//! Private names in expressions: `o.#x`, `o.#x = v`, `o.#m()`, `#x in o` (P2-CLS-CG, C7).
//! The key is a class context slot (`class_fields.rs` lists them):
//!
//! ```text
//! field #x     get:  GetKeyedProperty o, [acc = #x]          (TypeError if o has no #x)
//!              set:  SetKeyedProperty o, #x                  (TypeError if o has no #x)
//! method #m    get:  GetKeyedProperty o, [acc = brand]; acc = the #m slot
//! accessor #x  get:  brand check; CallProperty0 (the #x slot = getter), o
//! #x in o      TestIn #x (a method or accessor: the brand), [acc = o]
//! ```
//!
//! A field reference is a plain keyed `MemberRef`. A private method or accessor as an
//! assignment target is `MemberRef::PrivateBranded`: the brand check runs at each GetValue
//! and PutValue (after the right side), a write calls the setter, and a write to a method
//! (or a getter-only accessor, whose setter slot holds undefined) throws a TypeError.
//!
//! (M3-P4, E9) In direct eval code, a `#x` that only the caller's class declares resolves
//! `Dynamic`, and its kind comes from the `EvalContext` (`ScopeAnalysis::caller_private`). Its
//! class context slots are read by name (ep9_bytecode `eval_context.rs`):
//!
//! ```text
//! field #x     key:  LdaLookupSlot "#x"
//! method #m    env:  ResolveLookupSlot "#m"   (the class context that holds #m)
//!              brand LoadLookupSlotResolved env, "#.brand" (static: "#.static-brand")
//!              #m:   LoadLookupSlotResolved env, "#m"     (an accessor's setter: "#.set-x")
//! ```
//!
//! then the same keyed loads, brand checks, calls and `MemberRef`s as above.
//!
//! Callers: `expr.rs` (load, member reference, optional chain, call, `in`, `delete`, keys).

use ep9_bytecode::{FeedbackSlotKind, Register, RegisterList, RuntimeId};
use oxc_ast::ast::{PrivateFieldExpression, PrivateIdentifier, PrivateInExpression};

use super::class_fields::{BRAND, SETTER, STATIC_BRAND};
use super::expr::MemberRef;
use super::func::{FnGen, Gen};
use crate::scope::{
    BindingId, BindingKind, ImplicitRef, NameId, PrivateKind, Reference, Resolution,
};

/// (E9) A `#x` of the eval caller's class: its `Dynamic` reference, kind and static flag.
#[derive(Clone, Copy)]
struct CallerPrivate<'s> {
    reference: &'s Reference,
    kind: PrivateKind,
    is_static: bool,
}

/// (E9) Three temporaries for `LoadLookupSlotResolved`: the class context of a caller's `#x`
/// (from `ResolveLookupSlot`), a slot name, and `true` (class code is strict).
#[derive(Clone, Copy)]
struct CallerEnv(RegisterList);

/// A resolved `#x` reference.
#[derive(Clone, Copy)]
pub(crate) struct PrivateRef<'s> {
    reference: &'s Reference,
    binding: BindingId,
    hops: u32,
    kind: PrivateKind,
    is_static: bool,
}

impl<'a, 's> FnGen<'a, 's> {
    /// The binding of the `#x` at `field` (private names are always static bindings).
    fn private_ref(&self, field: &PrivateIdentifier<'_>) -> Gen<PrivateRef<'s>> {
        let reference = self.implicit(field.span.start, ImplicitRef::Private)?;
        let Resolution::Binding { binding, hops, .. } = reference.resolution else {
            return Err(super::internal(format!(
                "#{} at {} is not bound",
                field.name, field.span.start
            )));
        };
        let BindingKind::PrivateName { kind, is_static } = self.binding(binding).kind else {
            return Err(super::internal(format!(
                "#{} at {} is not a private name",
                field.name, field.span.start
            )));
        };
        Ok(PrivateRef {
            reference,
            binding,
            hops,
            kind,
            is_static,
        })
    }

    /// (E9) The `#x` at `field` if it is a private name of the eval caller's class.
    fn caller_private(&self, field: &PrivateIdentifier<'_>) -> Gen<Option<CallerPrivate<'s>>> {
        let reference = self.implicit(field.span.start, ImplicitRef::Private)?;
        if !matches!(reference.resolution, Resolution::Dynamic { .. }) {
            return Ok(None);
        }
        let name = self.sa.name(reference.name);
        let Some((kind, is_static)) = self.sa.caller_private(name) else {
            return Err(super::internal(format!(
                "{name} at {} is not a private name of the eval caller",
                field.span.start
            )));
        };
        Ok(Some(CallerPrivate {
            reference,
            kind,
            is_static,
        }))
    }

    /// (E9) `ResolveLookupSlot "#x"` into a new [`CallerEnv`].
    fn caller_env(&mut self, c: CallerPrivate<'s>) -> CallerEnv {
        let regs = self.temps(3);
        let first = regs.get(0).local_index().expect("temporaries are locals");
        let k = self.name_const(c.reference.name);
        self.b.lda_constant(k).star(regs.get(1));
        self.expr_pos(c.reference.span.start);
        self.runtime(
            RuntimeId::ResolveLookupSlot,
            RegisterList::new(first + 1, 1),
        );
        self.b.star(regs.get(0));
        self.b.lda_true().star(regs.get(2));
        CallerEnv(regs)
    }

    /// (E9) `acc = ` the slot `name` of the class context in `env`.
    fn caller_slot(&mut self, env: CallerEnv, name: &str, at: u32) {
        let k = self.str_const(name);
        self.b.lda_constant(k).star(env.0.get(1));
        self.expr_pos(at);
        self.runtime(RuntimeId::LoadLookupSlotResolved, env.0);
    }

    /// (E9) `acc = ` the brand of a caller's method or accessor `#x`.
    fn caller_brand(&mut self, c: CallerPrivate<'s>, env: CallerEnv) {
        let brand = if c.is_static { STATIC_BRAND } else { BRAND };
        self.caller_slot(env, brand, c.reference.span.start);
    }

    /// (E9) `acc = ` the method, or the getter (undefined when there is none), of `#x`.
    fn caller_method(&mut self, c: CallerPrivate<'s>, env: CallerEnv) {
        let name = self.sa.name(c.reference.name).to_owned();
        self.caller_slot(env, &name, c.reference.span.start);
    }

    /// (E9) `acc = ` the setter (undefined when there is none) of accessor `#x`.
    fn caller_setter(&mut self, c: CallerPrivate<'s>, env: CallerEnv) {
        let name = format!("{SETTER}{}", &self.sa.name(c.reference.name)[1..]);
        self.caller_slot(env, &name, c.reference.span.start);
    }

    /// (E9) `acc = ` the key that marks an object as having the caller's `#x` (the field's
    /// symbol, else the brand).
    fn caller_key(&mut self, c: CallerPrivate<'s>) {
        if c.kind == PrivateKind::Field {
            self.load_ref(c.reference, false);
        } else {
            let env = self.caller_env(c);
            self.caller_brand(c, env);
        }
    }

    /// (E9) `acc = obj.#x` for a caller's `#x` (PrivateGet, §7.3.31).
    fn caller_private_load(&mut self, c: CallerPrivate<'s>, obj: Register) {
        let at = c.reference.span.start;
        let m = self.mark();
        if c.kind == PrivateKind::Field {
            self.load_ref(c.reference, false);
            let s = self.slot(FeedbackSlotKind::LoadKeyed);
            self.expr_pos(at);
            self.b.get_keyed_property(obj, s);
        } else {
            let env = self.caller_env(c);
            self.caller_brand(c, env);
            let s = self.slot(FeedbackSlotKind::LoadKeyed);
            self.expr_pos(at);
            self.b.get_keyed_property(obj, s);
            self.caller_method(c, env);
            if c.kind != PrivateKind::Method {
                // A setter-only `#x` holds undefined: the call throws the TypeError.
                let getter = self.temp();
                self.b.star(getter);
                let s = self.slot(FeedbackSlotKind::Call);
                self.expr_pos(at);
                self.b.call_property0(getter, obj, s);
            }
        }
        self.release(m);
    }

    /// `acc = ` the key that marks an object as having `#x`: the private name symbol of a
    /// field, else the class brand.
    fn private_key(&mut self, r: PrivateRef<'s>) -> Gen {
        if r.kind == PrivateKind::Field {
            self.load_ref(r.reference, false);
        } else {
            let class = self.binding(r.binding).scope;
            let brand = self.brand(class, r.is_static)?;
            self.load_binding_raw(brand, r.hops, false);
        }
        Ok(())
    }

    /// The brand check of a method or accessor: GetKeyed throws a TypeError if `obj` does not
    /// have the brand.
    fn brand_check(&mut self, r: PrivateRef<'s>, obj: Register, at: u32) -> Gen {
        self.private_key(r)?;
        let s = self.slot(FeedbackSlotKind::LoadKeyed);
        self.expr_pos(at);
        self.b.get_keyed_property(obj, s);
        Ok(())
    }

    /// `acc = obj.#x` (PrivateGet, §7.3.31) with the object already in `obj`.
    fn private_load(&mut self, field: &PrivateIdentifier<'_>, obj: Register) -> Gen {
        if let Some(c) = self.caller_private(field)? {
            self.caller_private_load(c, obj);
            return Ok(());
        }
        let r = self.private_ref(field)?;
        let at = field.span.start;
        match r.kind {
            PrivateKind::Field => {
                self.load_ref(r.reference, false);
                let s = self.slot(FeedbackSlotKind::LoadKeyed);
                self.expr_pos(at);
                self.b.get_keyed_property(obj, s);
            }
            PrivateKind::Method => {
                self.brand_check(r, obj, at)?;
                self.load_binding_raw(r.binding, r.hops, false);
            }
            PrivateKind::Getter | PrivateKind::Setter | PrivateKind::GetterSetter => {
                // A setter-only `#x` holds undefined: the call throws the TypeError.
                self.brand_check(r, obj, at)?;
                let m = self.mark();
                let getter = self.temp();
                self.load_binding_raw(r.binding, r.hops, false);
                self.b.star(getter);
                let s = self.slot(FeedbackSlotKind::Call);
                self.expr_pos(at);
                self.b.call_property0(getter, obj, s);
                self.release(m);
            }
            PrivateKind::Accessor => return self.unsupported("class auto-accessors", field.span),
        }
        Ok(())
    }

    /// The object of `o.#x` into a new temporary; an optional link jumps to the chain end.
    fn private_object(&mut self, p: &'a PrivateFieldExpression<'a>) -> Gen<Register> {
        let obj = self.temp();
        self.expr(&p.object)?;
        if p.optional {
            let Some(end) = self.chain_end else {
                return Err(super::internal("optional link outside a chain"));
            };
            self.b.jump_if_undefined_or_null(end);
        }
        self.b.star(obj);
        Ok(obj)
    }

    /// `acc = o.#x` (also inside an optional chain).
    pub(crate) fn private_get(&mut self, p: &'a PrivateFieldExpression<'a>) -> Gen {
        let m = self.mark();
        let obj = self.private_object(p)?;
        self.private_load(&p.field, obj)?;
        self.release(m);
        Ok(())
    }

    /// The reference parts of `o.#x` as an assignment target: a keyed reference with the
    /// private name symbol as the key (the keyed opcodes send it to the private paths).
    pub(crate) fn private_member_ref(
        &mut self,
        p: &'a PrivateFieldExpression<'a>,
    ) -> Gen<MemberRef> {
        if let Some(c) = self.caller_private(&p.field)? {
            return self.caller_member_ref(p, c);
        }
        let r = self.private_ref(&p.field)?;
        let obj = self.private_object(p)?;
        match r.kind {
            PrivateKind::Field => {
                let key = self.temp();
                self.load_ref(r.reference, false);
                self.b.star(key);
                Ok(MemberRef::Keyed { obj, key })
            }
            PrivateKind::Accessor => self.unsupported("class auto-accessors", p.field.span),
            kind => {
                let brand = self.temp();
                self.private_key(r)?;
                self.b.star(brand);
                let get = self.temp();
                self.load_binding_raw(r.binding, r.hops, false);
                self.b.star(get);
                let set = if kind == PrivateKind::Method {
                    None
                } else {
                    let s = self.setter_binding(r.binding)?;
                    let set = self.temp();
                    self.load_binding_raw(s, r.hops, false);
                    self.b.star(set);
                    Some(set)
                };
                Ok(MemberRef::PrivateBranded {
                    obj,
                    brand,
                    get,
                    set,
                    name: self.binding(r.binding).name,
                    at: p.field.span.start,
                })
            }
        }
    }

    /// (E9) [`Self::private_member_ref`] for a caller's `#x`.
    fn caller_member_ref(
        &mut self,
        p: &'a PrivateFieldExpression<'a>,
        c: CallerPrivate<'s>,
    ) -> Gen<MemberRef> {
        let obj = self.private_object(p)?;
        if c.kind == PrivateKind::Field {
            let key = self.temp();
            self.load_ref(c.reference, false);
            self.b.star(key);
            return Ok(MemberRef::Keyed { obj, key });
        }
        let env = self.caller_env(c);
        let brand = self.temp();
        self.caller_brand(c, env);
        self.b.star(brand);
        let get = self.temp();
        self.caller_method(c, env);
        self.b.star(get);
        let set = if c.kind == PrivateKind::Method {
            None
        } else {
            let set = self.temp();
            self.caller_setter(c, env);
            self.b.star(set);
            Some(set)
        };
        Ok(MemberRef::PrivateBranded {
            obj,
            brand,
            get,
            set,
            name: c.reference.name,
            at: p.field.span.start,
        })
    }

    /// GetValue of a `MemberRef::PrivateBranded`: the brand check, then the method, or the
    /// getter's result (a setter-only accessor holds undefined: the call throws).
    pub(crate) fn private_branded_load(
        &mut self,
        obj: Register,
        brand: Register,
        get: Register,
        method: bool,
        at: u32,
    ) {
        let s = self.slot(FeedbackSlotKind::LoadKeyed);
        self.b.ldar(brand);
        self.expr_pos(at);
        self.b.get_keyed_property(obj, s);
        if method {
            self.b.ldar(get);
        } else {
            let s = self.slot(FeedbackSlotKind::Call);
            self.expr_pos(at);
            self.b.call_property0(get, obj, s);
        }
    }

    /// PutValue of the accumulator to a `MemberRef::PrivateBranded`: the brand check, then the
    /// setter call (a getter-only accessor holds undefined: the call throws), or for a method
    /// the TypeError of PrivateSet step 5 (ABI 5 runtime id 45 `ThrowTypeError` with the text
    /// "Private method '#m' is not writable", node v24's text; `name` is the private name with
    /// its `#`; it replaces ThrowConstAssignError, M2-P2-OPEN-ROLES). The accumulator is not
    /// kept.
    pub(crate) fn private_branded_store(
        &mut self,
        obj: Register,
        brand: Register,
        set: Option<Register>,
        name: NameId,
        at: u32,
    ) {
        let m = self.mark();
        let value = self.temp();
        self.b.star(value);
        let s = self.slot(FeedbackSlotKind::LoadKeyed);
        self.b.ldar(brand);
        self.expr_pos(at);
        self.b.get_keyed_property(obj, s);
        match set {
            Some(setter) => {
                let s = self.slot(FeedbackSlotKind::Call);
                self.expr_pos(at);
                self.b.call_property1(setter, obj, value, s);
            }
            None => {
                let message = self.temp();
                let text = format!("Private method '{}' is not writable", self.sa.name(name));
                let k = self.str_const(&text);
                self.b.lda_constant(k).star(message);
                self.runtime_with_at(
                    ep9_bytecode::RuntimeId::ThrowTypeError,
                    &[message],
                    Some(at),
                );
            }
        }
        self.release(m);
    }

    /// The callee of `o.#m(...)`: the function into `callee`, the receiver into `list[0]`
    /// (`list` has `1 + n` registers).
    pub(crate) fn private_callee(
        &mut self,
        p: &'a PrivateFieldExpression<'a>,
        callee: Register,
        list: RegisterList,
    ) -> Gen {
        let obj = list.get(0);
        self.expr(&p.object)?;
        if p.optional {
            let Some(end) = self.chain_end else {
                return Err(super::internal("optional link outside a chain"));
            };
            self.b.jump_if_undefined_or_null(end);
        }
        self.b.star(obj);
        self.private_load(&p.field, obj)?;
        self.b.star(callee);
        Ok(())
    }

    /// `#x in o` (§13.10.1): TestIn with the field's symbol or the class brand as the key.
    pub(crate) fn private_in(&mut self, p: &'a PrivateInExpression<'a>) -> Gen {
        if let Some(c) = self.caller_private(&p.left)? {
            let m = self.mark();
            let obj = self.expr_to_temp(&p.right)?;
            let key = self.temp();
            self.caller_key(c);
            self.b.star(key).ldar(obj);
            let s = self.slot(FeedbackSlotKind::HasKeyed);
            self.expr_pos(p.left.span.start);
            self.b.test_in(key, s);
            self.release(m);
            return Ok(());
        }
        let r = self.private_ref(&p.left)?;
        if r.kind == PrivateKind::Accessor {
            return self.unsupported("class auto-accessors", p.left.span);
        }
        let m = self.mark();
        let obj = self.expr_to_temp(&p.right)?;
        let key = self.temp();
        self.private_key(r)?;
        self.b.star(key).ldar(obj);
        let s = self.slot(FeedbackSlotKind::HasKeyed);
        self.expr_pos(p.left.span.start);
        self.b.test_in(key, s);
        self.release(m);
        Ok(())
    }

    /// `delete o.#x` is an early error (ECMA-262 §13.5.1.1), so only an AST that skipped the
    /// checks reaches it.
    pub(crate) fn private_delete(&mut self, p: &'a PrivateFieldExpression<'a>) -> Gen {
        Err(super::internal(format!(
            "delete of a private reference at {} (an early error)",
            p.span.start
        )))
    }

    /// A private name as a class element key into `r`: the value of its class context slot.
    pub(crate) fn private_key_into(&mut self, p: &'a PrivateIdentifier<'a>, r: Register) -> Gen {
        let b = self.sa.declared_at(p.span.start).ok_or_else(|| {
            super::internal(format!("no binding for #{} at {}", p.name, p.span.start))
        })?;
        self.load_binding_here(b, false);
        self.b.star(r);
        Ok(())
    }
}
