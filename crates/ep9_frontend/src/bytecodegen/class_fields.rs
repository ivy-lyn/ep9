//! Class fields, static blocks, private methods and the class constructor prologue
//! (P2-CLS-CG; C7, C8). The class context holds every private name and the synthetic
//! bindings of the scope pass (D2, `scope/collect.rs` `visit_class`):
//!
//! ```text
//! #x               field: a private name symbol (CreatePrivateName)
//! #m               method: the closure; getter: the getter (or undefined)
//! #.set-x          the setter of accessor #x (or undefined)
//! #.brand          the instance brand symbol (any instance private method or accessor)
//! #.static-brand   the static brand symbol (any static private method or accessor)
//! #.class-field-N  the key of the N-th computed field, after ToPropertyKey
//! ```
//!
//! ```text
//! class definition: CreatePrivateName per #x and per brand (class.rs, at scope entry)
//!                   private methods -> their slots; computed keys -> #.class-field-N
//!                   SetClassFieldsInitializer(ctor, instance initializer)
//!                   static brand: DefineKeyedOwnProperty ctor, brand, flags 0x02
//!                   static initializer: CallProperty0(fn, ctor)
//! instance initializer (this = the new object), in this order (InitializeInstanceElements):
//!                   DefineKeyedOwnProperty this, brand, flags 0x02 (value undefined)
//!                   per field: x = v   DefineNamedOwnProperty this, "x"
//!                              [k] = v DefineKeyedOwnProperty this, key, flags 0x01 (name)
//!                              #x = v  DefineKeyedOwnProperty this, #x, flags 0x02 (add)
//! static initializer (this = the constructor): static fields and static blocks, in order
//! base ctor entry:  RunClassFieldsInitializer(ctor, this), before the parameters (§10.2.2 6.b)
//! after super():    RunClassFieldsInitializer(ctor, this)
//! ```
//!
//! Every Run/Set call is emitted only for a class that has an instance initializer, so code
//! of classes without instance elements does not change.
//!
//! Callers: `class.rs` (definition, element checks), `func.rs` (`gen_function`,
//! `gen_default_constructor`) and `expr.rs` (`super_call`).

use ep9_bytecode::{FeedbackSlotKind, Register, RuntimeId, UnitString};
use oxc_ast::ast::{
    AccessorProperty, Class, ClassElement, MethodDefinition, MethodDefinitionKind,
    MethodDefinitionType, PrivateIdentifier, PropertyDefinition, PropertyDefinitionType,
    PropertyKey, StaticBlock,
};
use oxc_span::GetSpan;

use super::Job;
use super::func::{FnGen, Gen};
use super::literal::{self, PropertyName};
use crate::scope::{
    BindingId, BindingKind, ClassInfo, FunctionId, FunctionKind, PrivateKind, Reference,
    Resolution, ScopeId,
};

/// DefineKeyedOwnProperty flag bit 0: set the function name from the key.
pub(crate) const DEFINE_SET_NAME: u8 = 0x01;
/// DefineKeyedOwnProperty flag bit 1: a private add (TypeError if the object has the key).
pub(crate) const DEFINE_PRIVATE: u8 = 0x02;

/// The name prefix of the synthetic class bindings (D2).
const SYNTHETIC: &str = "#.";
const BRAND: &str = "#.brand";
const STATIC_BRAND: &str = "#.static-brand";
const SETTER: &str = "#.set-";
const FIELD_KEY: &str = "#.class-field-";

impl<'a> FnGen<'a, '_> {
    // ---- Element checks (class.rs `class_constructor`) ----

    /// A field (`x = v`, `static x`, `#x`, `[k] = v`).
    pub(crate) fn check_class_field(&self, p: &'a PropertyDefinition<'a>) -> Gen {
        if let Some(d) = p.decorators.first() {
            return self.unsupported("decorators", d.span);
        }
        if p.declare || p.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition {
            return self.unsupported("TypeScript declarations", p.span);
        }
        Ok(())
    }

    /// A `static { ... }` block.
    pub(crate) fn check_static_block(&self, s: &'a StaticBlock<'a>) -> Gen {
        let _ = s;
        Ok(())
    }

    /// An `accessor x` element (decorators proposal): not supported.
    pub(crate) fn check_auto_accessor(&self, p: &'a AccessorProperty<'a>) -> Gen {
        self.unsupported("class auto-accessors", p.span)
    }

    /// A method, getter or setter with a private name (`#m() {}`); `p` is its key. Async and
    /// generator forms are checked where the closure is made (`closure`).
    pub(crate) fn check_private_method(
        &self,
        m: &'a MethodDefinition<'a>,
        p: &'a PrivateIdentifier<'a>,
    ) -> Gen {
        if m.r#type == MethodDefinitionType::TSAbstractMethodDefinition {
            return self.unsupported("TypeScript declarations", p.span);
        }
        Ok(())
    }

    // ---- Class bindings ----

    /// The binding named `name` in class scope `scope`.
    fn class_binding(&self, scope: ScopeId, name: &str) -> Option<BindingId> {
        self.sa
            .scope(scope)
            .bindings
            .iter()
            .copied()
            .find(|&b| self.sa.name(self.binding(b).name) == name)
    }

    /// The instance (or static) brand of class scope `scope`.
    pub(crate) fn brand(&self, scope: ScopeId, is_static: bool) -> Gen<BindingId> {
        let name = if is_static { STATIC_BRAND } else { BRAND };
        self.class_binding(scope, name)
            .ok_or_else(|| super::internal(format!("no {name} in scope {}", scope.0)))
    }

    /// The setter slot `#.set-x` of the private accessor binding `accessor` (`#x`).
    pub(crate) fn setter_binding(&self, accessor: BindingId) -> Gen<BindingId> {
        let b = self.binding(accessor);
        let name = format!("{SETTER}{}", &self.sa.name(b.name)[1..]);
        self.class_binding(b.scope, &name)
            .ok_or_else(|| super::internal(format!("no {name} in scope {}", b.scope.0)))
    }

    /// The `#.class-field-N` bindings of class scope `scope`, in source order.
    pub(crate) fn computed_field_keys(&self, scope: ScopeId) -> Vec<BindingId> {
        self.sa
            .scope(scope)
            .bindings
            .iter()
            .copied()
            .filter(|&b| self.sa.name(self.binding(b).name).starts_with(FIELD_KEY))
            .collect()
    }

    /// The binding of a private name declared by element key `p`.
    fn declared_private(&self, p: &PrivateIdentifier<'_>) -> Gen<BindingId> {
        self.sa.declared_at(p.span.start).ok_or_else(|| {
            super::internal(format!("no binding for #{} at {}", p.name, p.span.start))
        })
    }

    /// The instance initializer of the class whose constructor is `f` (its outer scope is the
    /// class scope), if any.
    fn instance_initializer_of(&self, f: FunctionId) -> Option<FunctionId> {
        let scope = self.sa.function(f).outer_scope?;
        self.sa.scope(scope).class.as_ref()?.instance_initializer
    }

    // ---- Class definition (class.rs `class`) ----

    /// CreatePrivateName for each private field name and each brand of the class, into their
    /// class context slots. Methods and accessors need no symbol: their brand is the key. A
    /// field's description is `#x`; a brand's is the class name (`""` if anonymous), which the
    /// VM puts in its error text ("Receiver must be an instance of class C").
    pub(crate) fn create_private_names(
        &mut self,
        c: &'a Class<'a>,
        scope: ScopeId,
        class_name: Option<&UnitString>,
    ) -> Gen {
        for &b in &self.sa.scope(scope).bindings {
            let binding = self.binding(b);
            let name = self.sa.name(binding.name);
            let symbol = match binding.kind {
                BindingKind::PrivateName {
                    kind: PrivateKind::Field,
                    ..
                } => !name.starts_with(SYNTHETIC),
                BindingKind::PrivateName { .. } => name == BRAND || name == STATIC_BRAND,
                _ => false,
            };
            if !symbol {
                continue;
            }
            let at = binding.span.map_or(c.span.start, |s| s.start);
            let m = self.mark();
            let r = self.temp();
            let k = if name.starts_with(SYNTHETIC) {
                let description = class_name.cloned().unwrap_or_else(|| UnitString::new(""));
                self.b.string_constant(description)
            } else {
                self.str_const(name)
            };
            self.b.lda_constant(k).star(r);
            self.runtime_with_at(RuntimeId::CreatePrivateName, &[r], Some(at));
            self.store_binding_init(b);
            self.release(m);
        }
        Ok(())
    }

    /// A private method, getter or setter: its closure goes to its class context slot (a
    /// setter to `#.set-x`). `home` is the prototype or (static) the constructor.
    pub(crate) fn private_method_definition(
        &mut self,
        md: &'a MethodDefinition<'a>,
        p: &'a PrivateIdentifier<'a>,
        home: Register,
    ) -> Gen {
        let b = self.declared_private(p)?;
        let key = format!("#{}", p.name);
        let (name, slot) = match md.kind {
            MethodDefinitionKind::Get => (format!("get {key}"), b),
            MethodDefinitionKind::Set => (format!("set {key}"), self.setter_binding(b)?),
            _ => (key, b),
        };
        let fid = self.closure(&md.value, Some(UnitString::new(&name)))?;
        self.create_closure(fid);
        self.set_home_object(fid, home);
        self.store_binding_init(slot);
        Ok(())
    }

    /// The computed key of field `p`: ToPropertyKey once, at class definition, into its
    /// `#.class-field-N` slot `b`.
    pub(crate) fn computed_field_key(
        &mut self,
        p: &'a PropertyDefinition<'a>,
        b: BindingId,
    ) -> Gen {
        match (literal::property_name(&p.key), p.key.as_expression()) {
            (Some(PropertyName::Static(name)), _) => {
                let k = self.b.string_constant(name);
                self.b.lda_constant(k);
            }
            (None, Some(e)) => {
                self.expr(e)?;
                self.expr_pos(e.span().start);
                self.b.to_name();
            }
            (None, None) => {
                return Err(super::internal("a computed key that is not an expression"));
            }
        }
        self.store_binding_init(b);
        Ok(())
    }

    /// `[[HomeObject]]` of the closure in the accumulator (function `f`) when it uses `super`
    /// (or a direct eval may). The accumulator is kept.
    fn set_home_object(&mut self, f: FunctionId, home: Register) {
        if self.sa.function(f).home_object.is_none() {
            return;
        }
        let m = self.mark();
        let args = self.temps(2);
        self.b.star(args.get(0)).mov(home, args.get(1));
        self.runtime(RuntimeId::SetHomeObject, args);
        self.b.ldar(args.get(0));
        self.release(m);
    }

    /// The end of ClassDefinitionEvaluation (§15.7.14 steps 30-32), after the inner name:
    /// the instance initializer, the static brand, then the static initializer.
    pub(crate) fn class_definition_end(
        &mut self,
        c: &'a Class<'a>,
        info: &ClassInfo,
        scope: ScopeId,
        ctor: Register,
        proto: Register,
    ) -> Gen {
        let m = self.mark();
        if let Some(f) = info.instance_initializer {
            self.jobs.push(Job::DefaultConstructor {
                class: c,
                id: f,
                name: None,
            });
            let init = self.temp();
            self.create_closure(f);
            self.set_home_object(f, proto);
            self.b.star(init);
            self.runtime_with_at(
                RuntimeId::SetClassFieldsInitializer,
                &[ctor, init],
                Some(c.body.span.start),
            );
        }
        if let Some(brand) = self.class_binding(scope, STATIC_BRAND) {
            let key = self.temp();
            self.load_binding_here(brand, false);
            self.b.star(key).lda_undefined();
            let s = self.slot(FeedbackSlotKind::DefineKeyedOwn);
            self.expr_pos(c.body.span.start);
            self.b
                .define_keyed_own_property(ctor, key, DEFINE_PRIVATE, s);
        }
        if let Some(f) = info.static_initializer {
            self.jobs.push(Job::DefaultConstructor {
                class: c,
                id: f,
                name: None,
            });
            let init = self.temp();
            self.create_closure(f);
            self.set_home_object(f, ctor);
            self.b.star(init);
            let s = self.slot(FeedbackSlotKind::Call);
            self.expr_pos(c.body.span.start);
            self.b.call_property0(init, ctor, s);
        }
        self.release(m);
        Ok(())
    }

    // ---- Running the instance initializer ----

    /// RunClassFieldsInitializer(F, receiver), if F's class has an instance initializer.
    /// `load_f` puts F in the accumulator. Returns whether it emitted the call.
    fn run_fields_initializer(
        &mut self,
        f: FunctionId,
        load_f: impl FnOnce(&mut Self),
        receiver: Register,
        at: u32,
    ) -> bool {
        if self.instance_initializer_of(f).is_none() {
            return false;
        }
        let m = self.mark();
        let args = self.temps(2);
        load_f(self);
        self.b.star(args.get(0)).mov(receiver, args.get(1));
        self.expr_pos(at);
        self.runtime(RuntimeId::RunClassFieldsInitializer, args);
        self.release(m);
        true
    }

    /// The prologue of a base class constructor: InitializeInstanceElements before the
    /// parameters are bound (ECMA-262 §10.2.2 `[[Construct]]` step 6.b runs it before
    /// OrdinaryCallEvaluateBody). Called by `gen_function` before `prologue_and_params`.
    pub(crate) fn class_constructor_prologue(&mut self) -> Gen {
        if matches!(
            self.info.kind,
            FunctionKind::ClassConstructor | FunctionKind::DefaultConstructor
        ) {
            let at = self.info.span.start;
            let _ = self.run_fields_initializer(
                self.info.id,
                |g| {
                    g.b.lda_current_function();
                },
                Register::RECEIVER,
                at,
            );
        }
        Ok(())
    }

    /// The running constructor's class has an instance initializer.
    pub(crate) fn has_instance_initializer(&self) -> bool {
        self.instance_initializer_of(self.info.id).is_some()
    }

    /// The default derived constructor, after `super(...args)` left `this` in `this_reg`.
    pub(crate) fn default_derived_fields(&mut self, this_reg: Register, at: u32) {
        let _ = self.run_fields_initializer(
            self.info.id,
            |g| {
                g.b.lda_current_function();
            },
            this_reg,
            at,
        );
    }

    /// After `super()` bound `this` (held in `this_reg`): InitializeInstanceElements(this, F)
    /// (ECMA-262 §13.3.7.1 step 10), F = the derived constructor (`callee`, which can be the
    /// captured `.callee` of an arrow). The accumulator holds `this` afterwards.
    pub(crate) fn fields_after_super(&mut self, callee: &Reference, this_reg: Register) -> Gen {
        let Resolution::Binding { binding, .. } = callee.resolution else {
            return Err(super::internal("super() without a callee binding"));
        };
        let f = self.sa.scope(self.binding(binding).scope).function;
        let at = callee.span.start;
        if self.run_fields_initializer(f, |g| g.load_ref(callee, false), this_reg, at) {
            self.b.ldar(this_reg);
        }
        Ok(())
    }

    // ---- The initializer functions (func.rs `gen_default_constructor`) ----

    /// The body of an InstanceMembersInitializer or StaticMembersInitializer of `class`,
    /// after the prologue: the brand (instance), then the fields (and static blocks) of
    /// the kind in source order, then `return undefined`.
    pub(crate) fn members_initializer(&mut self, class: &'a Class<'a>) -> Gen {
        let is_static = self.info.kind == FunctionKind::StaticMembersInitializer;
        let Some(class_scope) = self.info.outer_scope else {
            return Err(super::internal("class initializer without a class scope"));
        };
        let this = Register::RECEIVER;
        if !is_static && let Some(brand) = self.class_binding(class_scope, BRAND) {
            let m = self.mark();
            let key = self.temp();
            self.load_binding_here(brand, false);
            self.b.star(key).lda_undefined();
            let s = self.slot(FeedbackSlotKind::DefineKeyedOwn);
            self.expr_pos(class.body.span.start);
            self.b
                .define_keyed_own_property(this, key, DEFINE_PRIVATE, s);
            self.release(m);
        }
        let field_keys = self.computed_field_keys(class_scope);
        let mut next_field = 0;
        for e in &class.body.body {
            match e {
                ClassElement::PropertyDefinition(p) => {
                    let key_slot =
                        if p.computed {
                            next_field += 1;
                            Some(field_keys.get(next_field - 1).copied().ok_or_else(|| {
                                super::internal("no slot for a computed field key")
                            })?)
                        } else {
                            None
                        };
                    if p.r#static == is_static {
                        self.enter(p.span)?;
                        self.stmt_pos(p.span);
                        let r = self.define_field(p, key_slot, this);
                        self.leave();
                        r?;
                    }
                }
                ClassElement::StaticBlock(block) if is_static => {
                    self.enter(block.span)?;
                    self.stmt_pos(block.span);
                    let r = self.static_block(block);
                    self.leave();
                    r?;
                }
                _ => {}
            }
        }
        self.b
            .set_throw_position(Some(class.body.span.end.saturating_sub(1)));
        self.b.lda_undefined().return_();
        Ok(())
    }

    /// DefineField(receiver, field) (§7.3.33): the key, the value (NamedEvaluation for an
    /// anonymous function), then the own define. `key_slot` holds a computed key.
    fn define_field(
        &mut self,
        p: &'a PropertyDefinition<'a>,
        key_slot: Option<BindingId>,
        this: Register,
    ) -> Gen {
        let m = self.mark();
        let at = p.key.span().start;
        let value = |g: &mut Self, name: Option<UnitString>| -> Gen {
            match &p.value {
                Some(v) => g.named_value(v, name),
                None => {
                    g.b.lda_undefined();
                    Ok(())
                }
            }
        };
        if let PropertyKey::PrivateIdentifier(id) = &p.key {
            let b = self.declared_private(id)?;
            let key = self.temp();
            self.load_binding_here(b, false);
            self.b.star(key);
            value(self, Some(UnitString::new(&format!("#{}", id.name))))?;
            let s = self.slot(FeedbackSlotKind::DefineKeyedOwn);
            self.expr_pos(at);
            self.b
                .define_keyed_own_property(this, key, DEFINE_PRIVATE, s);
        } else if let (false, Some(PropertyName::Static(name))) =
            (p.computed, literal::property_name(&p.key))
        {
            value(self, Some(name.clone()))?;
            let k = self.b.string_constant(name);
            let s = self.slot(FeedbackSlotKind::DefineNamedOwn);
            self.expr_pos(at);
            self.b.define_named_own_property(this, k, s);
        } else {
            let key = self.temp();
            match key_slot {
                Some(b) => {
                    self.load_binding_here(b, false);
                    self.b.star(key);
                }
                // A numeric literal key that is not a small integer: ToName now.
                None => self.property_key_into(&p.key, key)?,
            }
            let anonymous = p
                .value
                .as_ref()
                .is_some_and(|v| v.is_anonymous_function_definition());
            value(self, None)?;
            let flags = if anonymous { DEFINE_SET_NAME } else { 0 };
            let s = self.slot(FeedbackSlotKind::DefineKeyedOwn);
            self.expr_pos(at);
            self.b.define_keyed_own_property(this, key, flags, s);
        }
        self.release(m);
        Ok(())
    }

    /// A `static { ... }` block in the static initializer: its own var scope.
    fn static_block(&mut self, block: &'a StaticBlock<'a>) -> Gen {
        let scope = self.scope_of(block.scope_id.get(), block.span)?;
        let pushed = self.enter_scope(scope);
        self.hoist_functions(scope)?;
        self.statements(&block.body)?;
        self.exit_scope(scope, pushed);
        Ok(())
    }
}
