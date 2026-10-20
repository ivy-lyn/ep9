//! Classes v0: constructor, methods, getters, setters (static or not), `extends`.
//!
//! ```text
//! enter class scope (inner name in TDZ)
//! heritage -> r_heritage (or the hole)
//! CreateClosure(constructor) -> r_ctor
//! DefineClass(r_ctor, r_heritage) -> r_proto
//! per method, in order: key, CreateClosure,
//!   DefineClassMethod(home, key, fn) | DefineClassAccessor(home, key, get, set)
//! inner name = r_ctor; exit class scope; acc = r_ctor
//! ```
//!
//! Unsupported in v0: fields, private names, static blocks, `accessor`, decorators.

use ep9_bytecode::{Register, RuntimeId, UnitString};
use oxc_ast::ast::{Class, ClassElement, MethodDefinitionKind, MethodDefinitionType, PropertyKey};
use oxc_span::GetSpan;

use super::Job;
use super::func::{FnGen, Gen};
use super::literal::{self, PropertyName};

impl<'a> FnGen<'a, '_> {
    /// ClassDefinitionEvaluation. `name` is the class name (binding or NamedEvaluation).
    /// Leaves the constructor in the accumulator.
    pub(crate) fn class(&mut self, c: &'a Class<'a>, name: Option<UnitString>) -> Gen {
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
        self.runtime_with_at(
            RuntimeId::DefineClass,
            &[ctor, heritage],
            Some(c.span.start),
        );
        self.b.star(proto);
        self.class_methods(c, ctor, proto)?;
        if let Some(inner) = info.inner_name {
            self.b.ldar(ctor);
            self.store_binding_init(inner);
        }
        self.exit_scope(scope, pushed);
        self.b.ldar(ctor);
        self.release(m);
        Ok(())
    }

    /// Defines the methods, getters and setters in source order.
    #[inline(never)]
    fn class_methods(&mut self, c: &'a Class<'a>, ctor: Register, proto: Register) -> Gen {
        for e in &c.body.body {
            let ClassElement::MethodDefinition(md) = e else {
                continue;
            };
            if md.kind == MethodDefinitionKind::Constructor {
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
                ClassElement::StaticBlock(s) => {
                    return self.unsupported("class static blocks", s.span);
                }
                ClassElement::PropertyDefinition(p) => {
                    return self.unsupported("class fields", p.span);
                }
                ClassElement::AccessorProperty(p) => {
                    return self.unsupported("class auto-accessors", p.span);
                }
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
                        return self.unsupported("private names", p.span);
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
