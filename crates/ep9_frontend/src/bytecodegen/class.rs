//! Classes: constructor, methods, getters, setters (static or not), `extends`, fields,
//! private names, static blocks (P2-CLS-CG; the element code is in `class_fields.rs`).
//!
//! ```text
//! enter class scope (inner name in TDZ)
//! CreatePrivateName per field #x and per brand -> class context slots
//! heritage -> r_heritage (or the hole)
//! CreateClosure(constructor) -> r_ctor
//! (name from a runtime key: CallRuntime SetFunctionName(r_ctor, key), ABI 5 id 44)
//! DefineClass(r_ctor, r_heritage) -> r_proto
//! per element, in source order:
//!   method:         key, CreateClosure, DefineClassMethod(home, key, fn)
//!                   | DefineClassAccessor(home, key, get, set)
//!   private method: CreateClosure -> its class context slot (a setter: `#.set-x`)
//!   computed field: key, ToName -> `#.class-field-N`
//! inner name = r_ctor
//! SetClassFieldsInitializer(r_ctor, instance initializer)
//! static brand: DefineKeyedOwnProperty r_ctor, brand (private add)
//! static initializer: CallProperty0(fn, r_ctor)   (static fields and blocks, in order)
//! exit class scope; acc = r_ctor
//! ```
//!
//! Unsupported: `accessor` (auto-accessors) and decorators.

use ep9_bytecode::{Register, RuntimeId, UnitString};
use oxc_ast::ast::{Class, ClassElement, MethodDefinitionKind, MethodDefinitionType, PropertyKey};
use oxc_span::GetSpan;

use super::Job;
use super::func::{FnGen, Gen};
use super::literal::{self, PropertyName};
use crate::scope::ScopeId;

impl<'a> FnGen<'a, '_> {
    /// ClassDefinitionEvaluation. `name` is the class name (binding or NamedEvaluation).
    /// `name_key` instead holds a runtime property key (NamedEvaluation under a computed key)
    /// that names the class before its elements are defined. Leaves the constructor in the
    /// accumulator.
    pub(crate) fn class(
        &mut self,
        c: &'a Class<'a>,
        name: Option<UnitString>,
        name_key: Option<Register>,
    ) -> Gen {
        let constructor = self.class_constructor(c)?;
        let scope = self.scope_of(c.scope_id.get(), c.span)?;
        let Some(info) = self.sa.scope(scope).class.as_ref() else {
            return Err(super::internal(format!(
                "no class info at {}",
                c.span.start
            )));
        };
        let m = self.mark();
        let ctor = self.temp();
        let heritage = self.temp();
        let proto = self.temp();
        let pushed = self.enter_scope(scope);
        self.create_private_names(c, scope, name.as_ref())?;
        match &c.heritage {
            Some(h) => {
                self.expr(&h.expression)?;
            }
            None => {
                self.b.lda_the_hole();
            }
        }
        self.b.star(heritage);
        let id = match constructor {
            Some(m) => self.closure(&m.value, name.clone())?,
            None => {
                let Some(id) = info.default_constructor else {
                    return Err(super::internal("class without a constructor function"));
                };
                self.jobs.push(Job::DefaultConstructor {
                    class: c,
                    id,
                    name: name.clone(),
                });
                id
            }
        };
        self.create_closure(id);
        self.b.star(ctor);
        if let Some(key) = name_key {
            self.name_class_from_key(ctor, key);
        }
        self.runtime_with_at(
            RuntimeId::DefineClass,
            &[ctor, heritage],
            Some(c.span.start),
        );
        self.b.star(proto);
        self.class_methods(c, scope, ctor, proto)?;
        if let Some(inner) = info.inner_name {
            self.b.ldar(ctor);
            self.store_binding_init(inner);
        }
        self.class_definition_end(c, info, scope, ctor, proto)?;
        self.exit_scope(scope, pushed);
        self.b.ldar(ctor);
        self.release(m);
        Ok(())
    }

    /// SetFunctionName(F, className) (§15.7.14 step 15) for a class whose name is the
    /// property key in `key` (already ToPropertyKey'd; a symbol gives `"[description]"`), so
    /// the static fields and blocks see it and a `static name` member still replaces it. ABI 5
    /// runtime id 44 `SetFunctionName(function, key)` does it (it replaces the M2-P2 scratch
    /// object define with flag bit 0, M2-P2-C2-DEFER). The constructor is new and anonymous
    /// here, and no user code runs.
    fn name_class_from_key(&mut self, ctor: Register, key: Register) {
        self.runtime_with(RuntimeId::SetFunctionName, &[ctor, key]);
    }

    /// ClassElementEvaluation in source order: defines the methods, getters and setters,
    /// stores the private methods in the class context, and evaluates the computed field keys
    /// into their `#.class-field-N` slots.
    #[inline(never)]
    fn class_methods(
        &mut self,
        c: &'a Class<'a>,
        scope: ScopeId,
        ctor: Register,
        proto: Register,
    ) -> Gen {
        let field_keys = self.computed_field_keys(scope);
        let mut next_field = 0;
        for e in &c.body.body {
            let md = match e {
                ClassElement::MethodDefinition(md) => md,
                ClassElement::PropertyDefinition(p) if p.computed => {
                    let Some(&b) = field_keys.get(next_field) else {
                        return Err(super::internal("no slot for a computed field key"));
                    };
                    next_field += 1;
                    self.computed_field_key(p, b)?;
                    continue;
                }
                _ => continue,
            };
            if md.kind == MethodDefinitionKind::Constructor {
                continue;
            }
            if let PropertyKey::PrivateIdentifier(p) = &md.key {
                self.private_method_definition(md, p, if md.r#static { ctor } else { proto })?;
                continue;
            }
            let em = self.mark();
            let home = if md.r#static { ctor } else { proto };
            let accessor = matches!(
                md.kind,
                MethodDefinitionKind::Get | MethodDefinitionKind::Set
            );
            let args = self.temps(if accessor { 4 } else { 3 });
            self.b.mov(home, args.get(0));
            let static_name = if md.computed {
                None
            } else {
                literal::property_name(&md.key)
            };
            let fn_name = match static_name {
                Some(PropertyName::Static(key)) => {
                    let k = self.b.string_constant(key.clone());
                    self.b.lda_constant(k).star(args.get(1));
                    Some(match md.kind {
                        MethodDefinitionKind::Get => literal::prefixed("get ", &key),
                        MethodDefinitionKind::Set => literal::prefixed("set ", &key),
                        _ => key,
                    })
                }
                None => {
                    self.property_key_into(&md.key, args.get(1))?;
                    None
                }
            };
            let fid = self.closure(&md.value, fn_name)?;
            self.create_closure(fid);
            let id = match md.kind {
                MethodDefinitionKind::Get => {
                    self.b.star(args.get(2)).lda_undefined().star(args.get(3));
                    RuntimeId::DefineClassAccessor
                }
                MethodDefinitionKind::Set => {
                    self.b.star(args.get(3)).lda_undefined().star(args.get(2));
                    RuntimeId::DefineClassAccessor
                }
                _ => {
                    self.b.star(args.get(2));
                    RuntimeId::DefineClassMethod
                }
            };
            self.expr_pos(md.key.span().start);
            self.runtime(id, args);
            self.release(em);
        }
        Ok(())
    }

    /// The v0 support check; returns the explicit constructor, if any. Kept out of `class` so
    /// the recursive frame (heritage) stays small.
    #[inline(never)]
    fn class_constructor(
        &self,
        c: &'a Class<'a>,
    ) -> Gen<Option<&'a oxc_ast::ast::MethodDefinition<'a>>> {
        if let Some(d) = c.decorators.first() {
            return self.unsupported("decorators", d.span);
        }
        let mut constructor = None;
        for e in &c.body.body {
            match e {
                ClassElement::StaticBlock(s) => self.check_static_block(s)?,
                ClassElement::PropertyDefinition(p) => self.check_class_field(p)?,
                ClassElement::AccessorProperty(p) => self.check_auto_accessor(p)?,
                ClassElement::TSIndexSignature(_) => {}
                ClassElement::MethodDefinition(m) => {
                    if let Some(d) = m.decorators.first() {
                        return self.unsupported("decorators", d.span);
                    }
                    if m.r#type == MethodDefinitionType::TSAbstractMethodDefinition
                        || m.value.body.is_none()
                    {
                        return self.unsupported("TypeScript declarations", m.span);
                    }
                    if let PropertyKey::PrivateIdentifier(p) = &m.key {
                        self.check_private_method(m, p)?;
                    }
                    if m.kind == MethodDefinitionKind::Constructor {
                        constructor = Some(&**m);
                    }
                }
            }
        }
        Ok(constructor)
    }
}
