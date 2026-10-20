//! Destructuring: binding patterns (declarations, parameters, catch) and assignment patterns.
//!
//! ```text
//! object pattern: ToObject check, then per property: key -> (target ref) -> GetV -> default
//!                 -> bind; rest: CopyDataPropertiesExcluding(value, keys...)
//! array pattern:  GetIterator, then try { per element: (target ref) -> step -> default
//!                 -> bind } finally { if !done: IteratorClose }
//! ```

use ep9_bytecode::{FeedbackSlotKind, Label, Register, RegisterList, RuntimeId, UnitString};
use oxc_ast::ast::{
    ArrayAssignmentTarget, ArrayPattern, AssignmentTarget, AssignmentTargetMaybeDefault,
    AssignmentTargetPattern, AssignmentTargetProperty, BindingPattern, Expression,
    ObjectAssignmentTarget, ObjectPattern, PropertyKey,
};
use oxc_span::{GetSpan, Span};

use super::expr::Prepared;
use super::func::{FnGen, Gen};
use super::literal::{self, PropertyName};

/// How a binding pattern stores its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindMode {
    /// InitializeReferencedBinding: `let`, `const`, parameters, catch parameters.
    Init,
    /// PutValue through the scope chain: `var` declarators.
    Var,
}

/// One element target of a pattern.
#[derive(Clone, Copy)]
enum Target<'a> {
    Bind(&'a BindingPattern<'a>, BindMode),
    Assign(&'a AssignmentTarget<'a>),
}

impl<'a> Target<'a> {
    /// The name for NamedEvaluation of an anonymous function default value.
    fn name(self) -> Option<UnitString> {
        match self {
            Target::Bind(BindingPattern::BindingIdentifier(id), _) => {
                Some(UnitString::new(id.name.as_str()))
            }
            Target::Assign(AssignmentTarget::AssignmentTargetIdentifier(id)) => {
                Some(UnitString::new(id.name.as_str()))
            }
            _ => None,
        }
    }

    fn span(self) -> Span {
        match self {
            Target::Bind(p, _) => p.span(),
            Target::Assign(t) => t.span(),
        }
    }
}

/// `(target, default)` of an assignment pattern element.
fn split_maybe_default<'a>(
    t: &'a AssignmentTargetMaybeDefault<'a>,
) -> (&'a AssignmentTarget<'a>, Option<&'a Expression<'a>>) {
    match t {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(d) => (&d.binding, Some(&d.init)),
        other => (other.to_assignment_target(), None),
    }
}

/// `(pattern, default)` of a binding pattern element.
fn split_binding<'a>(
    p: &'a BindingPattern<'a>,
) -> (&'a BindingPattern<'a>, Option<&'a Expression<'a>>) {
    match p {
        BindingPattern::AssignmentPattern(a) => (&a.left, Some(&a.right)),
        other => (other, None),
    }
}

impl<'a, 's> FnGen<'a, 's> {
    /// Bind the value in `value` to `pattern`.
    pub(crate) fn bind_pattern(
        &mut self,
        pattern: &'a BindingPattern<'a>,
        value: Register,
        mode: BindMode,
    ) -> Gen {
        self.enter(pattern.span())?;
        let result = match pattern {
            BindingPattern::BindingIdentifier(id) => {
                self.b.ldar(value);
                self.bind_identifier(id, mode)
            }
            BindingPattern::ObjectPattern(o) => self.object_pattern(o, value, mode),
            BindingPattern::ArrayPattern(a) => self.array_pattern(a, value, mode),
            BindingPattern::AssignmentPattern(a) => {
                // Only reached for a top-level `x = d` pattern (parameters handle their own).
                let m = self.mark();
                let t = self.temp();
                let have = self.b.new_label();
                self.b.ldar(value).star(t);
                self.b.jump_if_not_undefined(have);
                self.named_value(&a.right, super::func::pattern_name(&a.left))?;
                self.b.star(t);
                self.b.bind(have);
                let r = self.bind_pattern(&a.left, t, mode);
                self.release(m);
                r
            }
        };
        self.leave();
        result
    }

    /// Destructuring assignment of the value in `value` to `pattern`.
    pub(crate) fn destructure_assign(
        &mut self,
        pattern: &'a AssignmentTargetPattern<'a>,
        value: Register,
    ) -> Gen {
        self.enter(pattern.span())?;
        let result = match pattern {
            AssignmentTargetPattern::ObjectAssignmentTarget(o) => self.object_assign(o, value),
            AssignmentTargetPattern::ArrayAssignmentTarget(a) => self.array_assign(a, value),
        };
        self.leave();
        result
    }

    /// RequireObjectCoercible(value): `ToObject` throws the TypeError for null and undefined.
    fn require_object_coercible(&mut self, value: Register, at: u32) {
        let m = self.mark();
        let t = self.temp();
        self.b.ldar(value);
        self.expr_pos(at);
        self.b.to_object(t);
        self.release(m);
    }

    /// Evaluate the target reference of an element before its value (assignment patterns).
    fn prepare_element(&mut self, t: Target<'a>) -> Gen<Option<Prepared<'a, 's>>> {
        match t {
            Target::Assign(a) if a.as_simple_assignment_target().is_some() => {
                Ok(Some(self.prepare_target(a)?))
            }
            _ => Ok(None),
        }
    }

    /// Apply the default if the accumulator is undefined, then store into the target.
    fn finish_element(
        &mut self,
        t: Target<'a>,
        prepared: Option<Prepared<'a, 's>>,
        default: Option<&'a Expression<'a>>,
    ) -> Gen {
        if let Some(init) = default {
            let have = self.b.new_label();
            self.b.jump_if_not_undefined(have);
            self.named_value(init, t.name())?;
            self.b.bind(have);
        }
        match (t, prepared) {
            (_, Some(p)) => self.put_prepared(&p, t.span().start, false),
            (Target::Bind(p, mode), None) => self.bind_value_in_acc(p, mode),
            (Target::Assign(a), None) => {
                let m = self.mark();
                let v = self.temp();
                self.b.star(v);
                let r = self.destructure_assign(a.to_assignment_target_pattern(), v);
                self.release(m);
                r
            }
        }
    }

    /// The key of one object pattern property: a constant name, or a computed key register.
    fn pattern_key(&mut self, key: &'a PropertyKey<'a>, computed: bool) -> Gen<PatternKey> {
        if !computed && let Some(PropertyName::Static(name)) = literal::property_name(key) {
            return Ok(PatternKey::Named(name));
        }
        Ok(PatternKey::Keyed(self.property_key_to_reg(key)?))
    }

    /// `acc = value[key]`.
    fn load_pattern_key(&mut self, value: Register, key: &PatternKey, at: u32) {
        self.expr_pos(at);
        match key {
            PatternKey::Named(name) => {
                let k = self.b.string_constant(name.clone());
                let s = self.slot(FeedbackSlotKind::LoadNamed);
                self.b.get_named_property(value, k, s);
            }
            PatternKey::Keyed(r) => {
                let s = self.slot(FeedbackSlotKind::LoadKeyed);
                self.b.ldar(*r).get_keyed_property(value, s);
            }
        }
    }

    /// Store a key into the exclusion list of an object rest.
    fn exclude_key(&mut self, key: &PatternKey, into: Register) {
        match key {
            PatternKey::Named(name) => {
                let k = self.b.string_constant(name.clone());
                self.b.lda_constant(k).star(into);
            }
            PatternKey::Keyed(r) => {
                self.b.mov(*r, into);
            }
        }
    }

    /// `acc = CopyDataPropertiesExcluding(value, keys...)` for an object rest.
    fn object_rest(&mut self, list: RegisterList, at: u32) {
        self.expr_pos(at);
        self.runtime(RuntimeId::CopyDataPropertiesExcluding, list);
    }

    fn object_pattern(&mut self, o: &'a ObjectPattern<'a>, value: Register, mode: BindMode) -> Gen {
        let m = self.mark();
        self.require_object_coercible(value, o.span.start);
        // Rest: list[0] = the source, list[1..] = the excluded keys.
        let list = o
            .rest
            .as_ref()
            .map(|_| self.temps(1 + o.properties.len() as u32));
        if let Some(list) = list {
            self.b.mov(value, list.get(0));
        }
        for (i, p) in o.properties.iter().enumerate() {
            let im = self.mark();
            let key = self.pattern_key(&p.key, p.computed)?;
            if let Some(list) = list {
                self.exclude_key(&key, list.get(1 + i as u32));
            }
            let (target, default) = split_binding(&p.value);
            self.load_pattern_key(value, &key, p.span.start);
            self.finish_element(Target::Bind(target, mode), None, default)?;
            self.release(im);
        }
        if let (Some(rest), Some(list)) = (&o.rest, list) {
            self.object_rest(list, rest.span.start);
            self.bind_value_in_acc(&rest.argument, mode)?;
        }
        self.release(m);
        Ok(())
    }

    fn object_assign(&mut self, o: &'a ObjectAssignmentTarget<'a>, value: Register) -> Gen {
        let m = self.mark();
        self.require_object_coercible(value, o.span.start);
        let list = o
            .rest
            .as_ref()
            .map(|_| self.temps(1 + o.properties.len() as u32));
        if let Some(list) = list {
            self.b.mov(value, list.get(0));
        }
        for (i, p) in o.properties.iter().enumerate() {
            let im = self.mark();
            match p {
                AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(id) => {
                    let key = PatternKey::Named(UnitString::new(id.binding.name.as_str()));
                    if let Some(list) = list {
                        self.exclude_key(&key, list.get(1 + i as u32));
                    }
                    let r = self.reference_of(&id.binding)?;
                    self.load_pattern_key(value, &key, id.span.start);
                    if let Some(init) = &id.init {
                        let have = self.b.new_label();
                        self.b.jump_if_not_undefined(have);
                        self.named_value(init, Some(UnitString::new(id.binding.name.as_str())))?;
                        self.b.bind(have);
                    }
                    self.store_ref(r, false);
                }
                AssignmentTargetProperty::AssignmentTargetPropertyProperty(pp) => {
                    let key = self.pattern_key(&pp.name, pp.computed)?;
                    if let Some(list) = list {
                        self.exclude_key(&key, list.get(1 + i as u32));
                    }
                    let (target, default) = split_maybe_default(&pp.binding);
                    let t = Target::Assign(target);
                    let prepared = self.prepare_element(t)?;
                    self.load_pattern_key(value, &key, pp.span.start);
                    self.finish_element(t, prepared, default)?;
                }
            }
            self.release(im);
        }
        if let (Some(rest), Some(list)) = (&o.rest, list) {
            let t = Target::Assign(&rest.target);
            let prepared = self.prepare_element(t)?;
            self.object_rest(list, rest.span.start);
            self.finish_element(t, prepared, None)?;
        }
        self.release(m);
        Ok(())
    }

    /// The shared loop of array patterns: `elements` gives each element's target and default
    /// (`None` for a hole); `rest` the rest target.
    fn array_destructure(
        &mut self,
        value: Register,
        at: u32,
        elements: Vec<Option<(Target<'a>, Option<&'a Expression<'a>>)>>,
        rest: Option<(Target<'a>, u32)>,
    ) -> Gen {
        let m = self.mark();
        let iter = self.temp();
        let next = self.temp();
        let done = self.temp();
        let v = self.temp();
        self.b.ldar(value);
        self.get_iterator(iter, next, at);
        self.try_finally(
            |g| {
                g.b.lda_false().star(done);
                for el in elements {
                    let em = g.mark();
                    let prepared = match el {
                        Some((t, _)) => g.prepare_element(t)?,
                        None => None,
                    };
                    g.array_step(iter, next, done, v, at);
                    if let Some((t, default)) = el {
                        g.b.ldar(v);
                        g.finish_element(t, prepared, default)?;
                    }
                    g.release(em);
                }
                if let Some((t, rest_at)) = rest {
                    let em = g.mark();
                    let prepared = g.prepare_element(t)?;
                    g.array_rest(iter, next, done, v, rest_at);
                    g.finish_element(t, prepared, None)?;
                    g.release(em);
                }
                Ok(())
            },
            |g, token| {
                let skip = g.b.new_label();
                g.b.ldar(done).jump_if_true(skip);
                g.close_iterator(iter, token);
                g.b.bind(skip);
                Ok(())
            },
        )?;
        self.release(m);
        Ok(())
    }

    /// One element step: `v = next value`, or undefined once the iterator is done. `done` is
    /// true while a step runs (a throw from `next` must not close the iterator).
    fn array_step(&mut self, iter: Register, next: Register, done: Register, v: Register, at: u32) {
        let (empty, have) = (self.b.new_label(), self.b.new_label());
        self.b.ldar(done).jump_if_true(empty);
        self.b.lda_true().star(done);
        self.iterator_step(iter, next, v, empty, at);
        self.b.lda_false().star(done);
        self.b.jump(have);
        self.b.bind(empty);
        self.b.lda_undefined().star(v);
        self.b.bind(have);
    }

    /// `acc = [...rest of the iterator]`.
    fn array_rest(&mut self, iter: Register, next: Register, done: Register, v: Register, at: u32) {
        let m = self.mark();
        let arr = self.temp();
        let idx = self.temp();
        let s = self.slot(FeedbackSlotKind::Literal);
        self.b.create_empty_array_literal(s).star(arr);
        self.b.lda_zero().star(idx);
        let store = self.slot(FeedbackSlotKind::StoreInArrayLiteral);
        let inc = self.slot(FeedbackSlotKind::BinaryOp);
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let end: Label = self.b.new_label();
        self.b.ldar(done).jump_if_true(end);
        self.b.lda_true().star(done);
        self.iterator_step(iter, next, v, end, at);
        self.b.lda_false().star(done);
        self.b.ldar(v).sta_in_array_literal(arr, idx, store);
        self.b.ldar(idx).inc(inc).star(idx);
        self.jump_loop(header);
        self.b.bind(end);
        self.loop_depth -= 1;
        self.b.ldar(arr);
        self.release(m);
    }

    fn array_pattern(&mut self, a: &'a ArrayPattern<'a>, value: Register, mode: BindMode) -> Gen {
        let elements = a
            .elements
            .iter()
            .map(|e| {
                e.as_ref().map(|p| {
                    let (t, d) = split_binding(p);
                    (Target::Bind(t, mode), d)
                })
            })
            .collect();
        let rest = a
            .rest
            .as_ref()
            .map(|r| (Target::Bind(&r.argument, mode), r.span.start));
        self.array_destructure(value, a.span.start, elements, rest)
    }

    fn array_assign(&mut self, a: &'a ArrayAssignmentTarget<'a>, value: Register) -> Gen {
        let elements = a
            .elements
            .iter()
            .map(|e| {
                e.as_ref().map(|t| {
                    let (t, d) = split_maybe_default(t);
                    (Target::Assign(t), d)
                })
            })
            .collect();
        let rest = a
            .rest
            .as_ref()
            .map(|r| (Target::Assign(&r.target), r.span.start));
        self.array_destructure(value, a.span.start, elements, rest)
    }
}

/// An object pattern key.
enum PatternKey {
    Named(UnitString),
    Keyed(Register),
}
