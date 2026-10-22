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
//! Callers: `expr.rs` (load, member reference, optional chain, call, `in`, `delete`, keys).

use ep9_bytecode::{FeedbackSlotKind, Register, RegisterList};
use oxc_ast::ast::{PrivateFieldExpression, PrivateIdentifier, PrivateInExpression};

use super::expr::MemberRef;
use super::func::{FnGen, Gen};
use crate::scope::{BindingId, BindingKind, ImplicitRef, PrivateKind, Reference, Resolution};

/// The TypeError text of PrivateSet step 5 (a store to a private method), thrown through ABI 5
/// runtime id 45 `ThrowTypeError`.
const PRIVATE_METHOD_NOT_WRITABLE: &str = "Private method is not writable";

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
                    at: p.field.span.start,
                })
            }
        }
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
    /// "Private method is not writable"; it replaces ThrowConstAssignError, M2-P2-OPEN-ROLES).
    /// The accumulator is not kept.
    pub(crate) fn private_branded_store(
        &mut self,
        obj: Register,
        brand: Register,
        set: Option<Register>,
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
                let k = self.str_const(PRIVATE_METHOD_NOT_WRITABLE);
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
