//! Expressions. Every `expr` leaves the value in the accumulator (G4).

use ep9_bytecode::{
    ConstIndex, FeedbackSlotKind, Label, Oddball, Register, RegisterList, RuntimeId, TypeOfLiteral,
    UnitString,
};
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, AssignmentExpression, AssignmentOperator,
    AssignmentTarget, BinaryExpression, BinaryOperator, CallExpression, ChainElement,
    ChainExpression, ComputedMemberExpression, ConditionalExpression, Expression,
    LogicalExpression, LogicalOperator, MemberExpression, NewExpression, ObjectExpression,
    ObjectPropertyKind, PropertyKey, PropertyKind, SimpleAssignmentTarget, StaticMemberExpression,
    TaggedTemplateExpression, TemplateLiteral, UnaryExpression, UnaryOperator, UpdateExpression,
    UpdateOperator,
};
use oxc_span::{GetSpan, Span};

use super::func::{FnGen, Gen};
use super::literal::{self, PropertyName};
use super::lookup::runs_no_code;
use crate::scope::{BindingLocation, ImplicitRef, NameId, Reference, Resolution};

/// A binary operator with a register form and a Smi form.
#[derive(Clone, Copy)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Sar,
    Shr,
}

fn arith_of(op: BinaryOperator) -> Option<Arith> {
    Some(match op {
        BinaryOperator::Addition => Arith::Add,
        BinaryOperator::Subtraction => Arith::Sub,
        BinaryOperator::Multiplication => Arith::Mul,
        BinaryOperator::Division => Arith::Div,
        BinaryOperator::Remainder => Arith::Mod,
        BinaryOperator::Exponential => Arith::Exp,
        BinaryOperator::BitwiseOR => Arith::BitOr,
        BinaryOperator::BitwiseXOR => Arith::BitXor,
        BinaryOperator::BitwiseAnd => Arith::BitAnd,
        BinaryOperator::ShiftLeft => Arith::Shl,
        BinaryOperator::ShiftRight => Arith::Sar,
        BinaryOperator::ShiftRightZeroFill => Arith::Shr,
        _ => return None,
    })
}

/// The target of a member expression whose object (and key) are already evaluated.
#[derive(Clone, Copy)]
pub(crate) enum MemberRef {
    Named {
        obj: Register,
        name: ConstIndex,
    },
    Keyed {
        obj: Register,
        key: Register,
    },
    SuperNamed {
        this: Register,
        home: Register,
        name: ConstIndex,
    },
    SuperKeyed {
        this: Register,
        home: Register,
        key: Register,
    },
    /// `o.#m` or `o.#a`, a private method or accessor (private.rs): `brand` holds the class
    /// brand, `get` the method or the getter, `set` the setter (`None` for a method). `name` is
    /// the private name (`#m`, for the TypeError text of a store to a method). `at` is the
    /// `#name` offset.
    PrivateBranded {
        obj: Register,
        brand: Register,
        get: Register,
        set: Option<Register>,
        name: NameId,
        at: u32,
    },
}

/// An assignment target with its reference parts evaluated.
pub(crate) enum Prepared<'a, 's> {
    Binding(&'s Reference),
    /// A dynamic name resolved before the right-hand side (`lookup.rs`
    /// `resolve_dynamic`): `regs` = [environment, name, value, strict].
    Dynamic {
        r: &'s Reference,
        regs: RegisterList,
    },
    Member(MemberRef),
    Pattern(&'a oxc_ast::ast::AssignmentTargetPattern<'a>),
}

/// The expression is a direct `eval(...)` call (same rule as the scope pass).
pub(crate) fn is_direct_eval(call: &CallExpression<'_>) -> bool {
    if call.optional {
        return false;
    }
    let mut callee = &call.callee;
    while let Expression::ParenthesizedExpression(p) = callee {
        callee = &p.expression;
    }
    matches!(callee, Expression::Identifier(id) if id.name == "eval")
}

/// The expression cannot write a register local: literals, identifiers, `this`.
fn write_free(e: &Expression<'_>) -> bool {
    match e {
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::Identifier(_)
        | Expression::ThisExpression(_) => true,
        Expression::ParenthesizedExpression(p) => write_free(&p.expression),
        Expression::TemplateLiteral(t) => t.expressions.is_empty(),
        _ => false,
    }
}

/// An int32 literal (not -0) for the Smi forms.
fn smi_literal(e: &Expression<'_>) -> Option<i32> {
    match e.without_parentheses() {
        Expression::NumericLiteral(n) => literal::as_smi(n.value),
        _ => None,
    }
}

impl<'a, 's> FnGen<'a, 's> {
    pub(crate) fn expr(&mut self, e: &'a Expression<'a>) -> Gen {
        self.enter(e.span())?;
        let result = self.expr_inner(e, None);
        self.leave();
        result
    }

    /// NamedEvaluation: an anonymous function or class gets `name`.
    pub(crate) fn named_value(&mut self, e: &'a Expression<'a>, name: Option<UnitString>) -> Gen {
        self.enter(e.span())?;
        let result = self.expr_inner(e, name);
        self.leave();
        result
    }

    /// NamedEvaluation of `e` under a computed key: the name is the property key in `key`
    /// (already ToPropertyKey'd). An anonymous class gets it inside ClassDefinitionEvaluation,
    /// before its static elements run, and the result is false. Any other `e` is evaluated
    /// without a name, and the result says whether it is an anonymous function definition:
    /// the caller then names it at the define (flag bit 0), since a function cannot observe
    /// its name before that.
    pub(crate) fn keyed_named_value(&mut self, e: &'a Expression<'a>, key: Register) -> Gen<bool> {
        if let Expression::ClassExpression(c) = e.without_parentheses()
            && c.id.is_none()
        {
            self.enter(e.span())?;
            let result = self.class(c, None, Some(key));
            self.leave();
            result?;
            return Ok(false);
        }
        let anonymous = e.is_anonymous_function_definition();
        self.named_value(e, None)?;
        Ok(anonymous)
    }

    /// Evaluate for side effects only.
    pub(crate) fn expr_effect(&mut self, e: &'a Expression<'a>) -> Gen {
        match e {
            Expression::AssignmentExpression(a) => {
                self.enter(e.span())?;
                let r = self.assignment(a, false);
                self.leave();
                r
            }
            Expression::UpdateExpression(u) => {
                self.enter(e.span())?;
                let r = self.update(u, false);
                self.leave();
                r
            }
            Expression::SequenceExpression(s) => {
                self.enter(e.span())?;
                let mut r = Ok(());
                for x in &s.expressions {
                    r = self.expr_effect(x);
                    if r.is_err() {
                        break;
                    }
                }
                self.leave();
                r
            }
            Expression::ParenthesizedExpression(p) => self.expr_effect(&p.expression),
            _ => self.expr(e),
        }
    }

    /// Evaluate into a new temporary.
    pub(crate) fn expr_to_temp(&mut self, e: &'a Expression<'a>) -> Gen<Register> {
        let r = self.temp();
        self.expr(e)?;
        self.b.star(r);
        Ok(r)
    }

    /// Evaluate into `r`.
    pub(crate) fn expr_into(&mut self, e: &'a Expression<'a>, r: Register) -> Gen {
        self.expr(e)?;
        self.b.star(r);
        Ok(())
    }

    /// A plain local (register or parameter, no hole check) that `e` reads.
    fn local_register(&self, e: &Expression<'_>) -> Option<Register> {
        let Expression::Identifier(id) = e.without_parentheses() else {
            return None;
        };
        let r = self.sa.reference(id.reference_id.get()?)?;
        let Resolution::Binding {
            binding,
            hole_check: false,
            ..
        } = r.resolution
        else {
            return None;
        };
        match self.binding(binding).location {
            BindingLocation::Register(i) => Some(Register::local(i)),
            BindingLocation::Parameter(i) => Some(Register::parameter(i)),
            _ => None,
        }
    }

    /// `e` as a register operand. A plain local is used directly when `later` cannot write it
    /// (G4); else a new temporary.
    fn operand_reg(&mut self, e: &'a Expression<'a>, later: &Expression<'_>) -> Gen<Register> {
        if write_free(later)
            && let Some(r) = self.local_register(e)
        {
            return Ok(r);
        }
        self.expr_to_temp(e)
    }

    fn expr_inner(&mut self, e: &'a Expression<'a>, name: Option<UnitString>) -> Gen {
        match e {
            Expression::BooleanLiteral(b) => {
                if b.value {
                    self.b.lda_true();
                } else {
                    self.b.lda_false();
                }
                Ok(())
            }
            Expression::NullLiteral(_) => {
                self.b.lda_null();
                Ok(())
            }
            Expression::NumericLiteral(n) => {
                self.number(n.value);
                Ok(())
            }
            Expression::BigIntLiteral(n) => {
                let k = self
                    .b
                    .bigint_constant(literal::bigint_from_decimal(n.value.as_str()));
                self.b.lda_constant(k);
                Ok(())
            }
            Expression::RegExpLiteral(r) => {
                let k = self.str_const(r.regex.pattern.text.as_str());
                let s = self.slot(FeedbackSlotKind::Literal);
                self.expr_pos(r.span.start);
                self.b
                    .create_reg_exp_literal(k, s, literal::regexp_flags(r.regex.flags));
                Ok(())
            }
            Expression::StringLiteral(s) => {
                let k = self
                    .b
                    .string_constant(literal::js_string(s.value.encode_utf16()));
                self.b.lda_constant(k);
                Ok(())
            }
            Expression::TemplateLiteral(t) => self.template(t),
            Expression::Identifier(id) => {
                let r = self.reference_of(id)?;
                self.load_ref(r, false);
                Ok(())
            }
            Expression::Super(s) => self.unsupported("bare super", s.span),
            Expression::ArrayExpression(a) => self.array(a),
            Expression::ArrowFunctionExpression(f) => {
                if f.r#async {
                    self.async_closure(f.span)?;
                }
                let id = self.function_id(f.span)?;
                self.jobs.push(super::Job::Arrow { node: f, id, name });
                self.create_closure(id);
                Ok(())
            }
            Expression::AssignmentExpression(a) => self.assignment(a, true),
            Expression::AwaitExpression(a) => self.await_expr(a),
            Expression::BinaryExpression(b) => self.binary(b),
            Expression::CallExpression(c) => self.call(c),
            Expression::ChainExpression(c) => self.chain(c),
            Expression::ClassExpression(c) => {
                let name =
                    c.id.as_ref()
                        .map(|id| UnitString::new(id.name.as_str()))
                        .or(name);
                self.class(c, name, None)
            }
            Expression::ConditionalExpression(c) => self.conditional(c),
            Expression::FunctionExpression(f) => {
                let name =
                    f.id.as_ref()
                        .map(|id| UnitString::new(id.name.as_str()))
                        .or(name);
                let id = self.closure(f, name)?;
                self.create_closure(id);
                Ok(())
            }
            Expression::ImportExpression(i) => self.import_call(i),
            Expression::LogicalExpression(l) => self.logical(l),
            Expression::NewExpression(n) => self.new_expr(n),
            Expression::ObjectExpression(o) => self.object(o),
            Expression::ParenthesizedExpression(p) => self.expr_inner(&p.expression, name),
            Expression::SequenceExpression(s) => {
                let (last, rest) = s.expressions.split_last().expect("non-empty sequence");
                for x in rest {
                    self.expr_effect(x)?;
                }
                self.expr(last)
            }
            Expression::TaggedTemplateExpression(t) => self.tagged_template(t),
            Expression::ThisExpression(t) => {
                let r = self.implicit(t.span.start, ImplicitRef::This)?;
                self.load_ref(r, false);
                Ok(())
            }
            Expression::UnaryExpression(u) => self.unary(u),
            Expression::UpdateExpression(u) => self.update(u, true),
            Expression::YieldExpression(y) => self.yield_expr(y),
            Expression::PrivateInExpression(p) => self.private_in(p),
            Expression::ImportMeta(m) => self.import_meta(m),
            Expression::NewTarget(t) => {
                let r = self.implicit(t.span.start, ImplicitRef::NewTarget)?;
                self.load_ref(r, false);
                Ok(())
            }
            Expression::JSXElement(j) => self.unsupported("JSX", j.span),
            Expression::JSXFragment(j) => self.unsupported("JSX", j.span),
            Expression::TSAsExpression(t) => self.expr(&t.expression),
            Expression::TSSatisfiesExpression(t) => self.expr(&t.expression),
            Expression::TSTypeAssertion(t) => self.expr(&t.expression),
            Expression::TSNonNullExpression(t) => self.expr(&t.expression),
            Expression::TSInstantiationExpression(t) => self.expr(&t.expression),
            Expression::V8IntrinsicExpression(v) => self.unsupported("V8 intrinsics", v.span),
            Expression::StaticMemberExpression(m) => self.static_member(m),
            Expression::ComputedMemberExpression(m) => self.computed_member(m),
            Expression::PrivateFieldExpression(p) => self.private_get(p),
        }
    }

    // ---- Literals ----

    /// `acc = v` for a small integer.
    pub(crate) fn smi(&mut self, v: i32) {
        if v == 0 {
            self.b.lda_zero();
        } else {
            self.b.lda_smi(v);
        }
    }

    pub(crate) fn number(&mut self, v: f64) {
        match literal::as_smi(v) {
            Some(0) => {
                self.b.lda_zero();
            }
            Some(i) => {
                self.b.lda_smi(i);
            }
            None => {
                let k = self.b.number_constant(v);
                self.b.lda_constant(k);
            }
        }
    }

    fn template(&mut self, t: &'a TemplateLiteral<'a>) -> Gen {
        let cooked = |i: usize| -> UnitString {
            t.quasis[i].value.cooked.map_or_else(
                || UnitString::new(""),
                |c| literal::js_string(c.encode_utf16()),
            )
        };
        if t.expressions.is_empty() {
            let k = self.b.string_constant(cooked(0));
            self.b.lda_constant(k);
            return Ok(());
        }
        let m = self.mark();
        let acc = self.temp();
        let mut have = false;
        let first = cooked(0);
        if !first.is_empty() {
            let k = self.b.string_constant(first);
            self.b.lda_constant(k).star(acc);
            have = true;
        }
        for (i, e) in t.expressions.iter().enumerate() {
            self.expr(e)?;
            self.expr_pos(e.span().start);
            self.b.to_string();
            if have {
                let s = self.slot(FeedbackSlotKind::BinaryOp);
                self.b.add(acc, s);
            }
            have = true;
            let q = cooked(i + 1);
            if !q.is_empty() {
                self.b.star(acc);
                let k = self.b.string_constant(q);
                let s = self.slot(FeedbackSlotKind::BinaryOp);
                self.b.lda_constant(k).add(acc, s);
            }
            if i + 1 < t.expressions.len() {
                self.b.star(acc);
            }
        }
        self.release(m);
        Ok(())
    }

    fn tagged_template(&mut self, t: &'a TaggedTemplateExpression<'a>) -> Gen {
        let parts = t
            .quasi
            .quasis
            .iter()
            .map(|q| {
                (
                    UnitString::new(&literal::template_raw(q.value.raw.as_str())),
                    q.value.cooked.map(|c| literal::js_string(c.encode_utf16())),
                )
            })
            .collect();
        let m = self.mark();
        let callee = self.temp();
        let n = 1 + t.quasi.expressions.len() as u32;
        let receiver = self.callee_and_receiver(&t.tag, callee, n)?;
        let args = match receiver {
            Some(list) => list,
            None => self.temps(n),
        };
        let first = if receiver.is_some() { 1 } else { 0 };
        let k = self.b.template_object(parts);
        let s = self.slot(FeedbackSlotKind::Literal);
        self.b.get_template_object(k, s).star(args.get(first));
        for (i, e) in t.quasi.expressions.iter().enumerate() {
            self.expr_into(e, args.get(first + 1 + i as u32))?;
        }
        self.expr_pos(t.span.start);
        self.emit_call(callee, args, receiver.is_some(), false);
        self.release(m);
        Ok(())
    }

    /// The constant for a boilerplate element or property value, if `e` is a constant.
    fn boilerplate_value(&mut self, e: &'a Expression<'a>, depth: u32) -> Option<ConstIndex> {
        match e {
            Expression::NumericLiteral(n) => Some(self.b.number_constant(n.value)),
            Expression::StringLiteral(s) => Some(
                self.b
                    .string_constant(literal::js_string(s.value.encode_utf16())),
            ),
            Expression::BooleanLiteral(b) => Some(self.b.oddball_constant(if b.value {
                Oddball::True
            } else {
                Oddball::False
            })),
            Expression::NullLiteral(_) => Some(self.b.oddball_constant(Oddball::Null)),
            Expression::BigIntLiteral(n) => Some(
                self.b
                    .bigint_constant(literal::bigint_from_decimal(n.value.as_str())),
            ),
            Expression::UnaryExpression(u)
                if u.operator == UnaryOperator::UnaryNegation
                    && matches!(u.argument, Expression::NumericLiteral(_)) =>
            {
                let Expression::NumericLiteral(n) = &u.argument else {
                    unreachable!()
                };
                Some(self.b.number_constant(-n.value))
            }
            Expression::ArrayExpression(a) if depth < 4 => {
                let mut elems = Vec::with_capacity(a.elements.len());
                for el in &a.elements {
                    let v = match el {
                        ArrayExpressionElement::Elision(_) => {
                            self.b.oddball_constant(Oddball::Hole)
                        }
                        ArrayExpressionElement::SpreadElement(_) => return None,
                        other => self.boilerplate_value(other.to_expression(), depth + 1)?,
                    };
                    elems.push(Some(v));
                }
                Some(self.b.array_boilerplate(elems))
            }
            Expression::ObjectExpression(o) if depth < 4 => {
                let mut props = Vec::with_capacity(o.properties.len());
                let mut seen: Vec<UnitString> = Vec::new();
                for p in &o.properties {
                    let ObjectPropertyKind::ObjectProperty(p) = p else {
                        return None;
                    };
                    if p.kind != PropertyKind::Init || p.computed || p.method {
                        return None;
                    }
                    let key = literal::property_name(&p.key)?;
                    let PropertyName::Static(key) = key;
                    if literal::is_proto_key(&key) || seen.contains(&key) {
                        return None;
                    }
                    let v = self.boilerplate_value(&p.value, depth + 1)?;
                    seen.push(key.clone());
                    let k = self.b.string_constant(key);
                    props.push((k, Some(v)));
                }
                Some(self.b.object_boilerplate(props))
            }
            _ => None,
        }
    }

    fn array(&mut self, a: &'a ArrayExpression<'a>) -> Gen {
        let elems = &a.elements;
        if elems.is_empty() {
            let s = self.slot(FeedbackSlotKind::Literal);
            self.b.create_empty_array_literal(s);
            return Ok(());
        }
        if let [ArrayExpressionElement::SpreadElement(s)] = elems.as_slice() {
            self.expr(&s.argument)?;
            self.expr_pos(s.span.start);
            self.b.create_array_from_iterable();
            return Ok(());
        }
        let first_spread = elems
            .iter()
            .position(|e| matches!(e, ArrayExpressionElement::SpreadElement(_)))
            .unwrap_or(elems.len());
        let mut consts = Vec::with_capacity(first_spread);
        let mut computed = Vec::new();
        let mut nested = false;
        for (i, el) in elems[..first_spread].iter().enumerate() {
            match el {
                ArrayExpressionElement::Elision(_) => {
                    consts.push(Some(self.b.oddball_constant(Oddball::Hole)));
                }
                other => {
                    let e = other.to_expression();
                    let v = self.boilerplate_value(e, 0);
                    nested |= v.is_some()
                        && matches!(
                            e,
                            Expression::ArrayExpression(_) | Expression::ObjectExpression(_)
                        );
                    if v.is_none() {
                        computed.push(i);
                    }
                    consts.push(v);
                }
            }
        }
        let k = self.b.array_boilerplate(consts);
        let s = self.slot(FeedbackSlotKind::Literal);
        self.expr_pos(a.span.start);
        self.b.create_array_literal(k, s, u8::from(!nested));
        if computed.is_empty() && first_spread == elems.len() {
            return Ok(());
        }
        let m = self.mark();
        let arr = self.temp();
        let idx = self.temp();
        self.b.star(arr);
        let store = self.slot(FeedbackSlotKind::StoreInArrayLiteral);
        for i in computed {
            self.smi(i as i32);
            self.b.star(idx);
            self.expr(elems[i].to_expression())?;
            self.b.sta_in_array_literal(arr, idx, store);
        }
        if first_spread < elems.len() {
            self.smi(first_spread as i32);
            self.b.star(idx);
            let inc = self.slot(FeedbackSlotKind::BinaryOp);
            for el in &elems[first_spread..] {
                match el {
                    ArrayExpressionElement::SpreadElement(sp) => {
                        self.expr(&sp.argument)?;
                        self.spread_into_array(arr, idx, sp.span.start);
                    }
                    ArrayExpressionElement::Elision(_) => {
                        self.b.ldar(idx).inc(inc).star(idx);
                    }
                    other => {
                        self.expr(other.to_expression())?;
                        self.b.sta_in_array_literal(arr, idx, store);
                        self.b.ldar(idx).inc(inc).star(idx);
                    }
                }
            }
            if matches!(elems.last(), Some(ArrayExpressionElement::Elision(_))) {
                let k = self.str_const("length");
                let s = self.slot(FeedbackSlotKind::StoreNamed);
                self.b.ldar(idx).set_named_property(arr, k, s);
            }
        }
        self.b.ldar(arr);
        self.release(m);
        Ok(())
    }

    /// Append the elements of the iterable in the accumulator to `arr` at `idx`.
    pub(crate) fn spread_into_array(&mut self, arr: Register, idx: Register, at: u32) {
        let m = self.mark();
        let iter = self.temp();
        let next = self.temp();
        let value = self.temp();
        self.get_iterator(iter, next, at);
        self.loop_depth += 1;
        let header = self.b.bound_label();
        let done = self.b.new_label();
        self.iterator_step(iter, next, value, done, at);
        let store = self.slot(FeedbackSlotKind::StoreInArrayLiteral);
        let inc = self.slot(FeedbackSlotKind::BinaryOp);
        self.b.ldar(value).sta_in_array_literal(arr, idx, store);
        self.b.ldar(idx).inc(inc).star(idx);
        self.jump_loop(header);
        self.b.bind(done);
        self.loop_depth -= 1;
        self.release(m);
    }

    fn object(&mut self, o: &'a ObjectExpression<'a>) -> Gen {
        let props = &o.properties;
        if props.is_empty() {
            self.b.create_empty_object_literal();
            return Ok(());
        }
        // The boilerplate: a prefix of plain properties with distinct static keys.
        let mut entries: Vec<(ConstIndex, Option<ConstIndex>)> = Vec::new();
        let mut deferred: Vec<(usize, UnitString)> = Vec::new();
        let mut seen: Vec<UnitString> = Vec::new();
        let mut nested = false;
        let mut i = 0;
        while i < props.len() {
            let ObjectPropertyKind::ObjectProperty(p) = &props[i] else {
                break;
            };
            if p.kind != PropertyKind::Init || p.computed {
                break;
            }
            let Some(PropertyName::Static(key)) = literal::property_name(&p.key) else {
                break;
            };
            if literal::is_proto_key(&key) || seen.contains(&key) {
                break;
            }
            let v = if p.method {
                None
            } else {
                self.boilerplate_value(&p.value, 0)
            };
            nested |= v.is_some()
                && matches!(
                    p.value,
                    Expression::ArrayExpression(_) | Expression::ObjectExpression(_)
                );
            let k = self.b.string_constant(key.clone());
            if v.is_none() {
                deferred.push((i, key.clone()));
            }
            entries.push((k, v));
            seen.push(key);
            i += 1;
        }
        if !entries.is_empty() && deferred.is_empty() && i == props.len() {
            let k = self.b.object_boilerplate(entries);
            let s = self.slot(FeedbackSlotKind::Literal);
            self.expr_pos(o.span.start);
            self.b.create_object_literal(k, s, u8::from(!nested));
            return Ok(());
        }
        let m = self.mark();
        let obj = self.temp();
        self.expr_pos(o.span.start);
        if entries.is_empty() {
            if let ObjectPropertyKind::SpreadProperty(sp) = &props[0] {
                let src = self.expr_to_temp(&sp.argument)?;
                let s = self.slot(FeedbackSlotKind::CloneObject);
                self.expr_pos(sp.span.start);
                self.b.clone_object(src, 0, s).star(obj);
                i = 1;
            } else {
                self.b.create_empty_object_literal().star(obj);
            }
        } else {
            let k = self.b.object_boilerplate(entries);
            let s = self.slot(FeedbackSlotKind::Literal);
            self.b
                .create_object_literal(k, s, u8::from(!nested))
                .star(obj);
        }
        for (j, key) in deferred {
            let ObjectPropertyKind::ObjectProperty(p) = &props[j] else {
                unreachable!("deferred entries are properties");
            };
            self.object_value(p, obj, Some(key.clone()))?;
            let k = self.b.string_constant(key);
            let s = self.slot(FeedbackSlotKind::DefineNamedOwn);
            self.b.define_named_own_property(obj, k, s);
        }
        for p in &props[i..] {
            match p {
                ObjectPropertyKind::SpreadProperty(sp) => {
                    let src = self.expr_to_temp(&sp.argument)?;
                    self.runtime_with_at(
                        RuntimeId::CopyDataProperties,
                        &[obj, src],
                        Some(sp.span.start),
                    );
                }
                ObjectPropertyKind::ObjectProperty(p) => self.object_property(p, obj)?,
            }
        }
        self.b.ldar(obj);
        self.release(m);
        Ok(())
    }

    /// `acc = value of p` (a method closure gets its home object when it uses `super`).
    fn object_value(
        &mut self,
        p: &'a oxc_ast::ast::ObjectProperty<'a>,
        obj: Register,
        name: Option<UnitString>,
    ) -> Gen {
        if p.method || p.kind != PropertyKind::Init {
            let Expression::FunctionExpression(f) = &p.value else {
                return Err(super::internal("method without a function"));
            };
            let id = self.closure(f, name)?;
            self.create_closure(id);
            if self.sa.function(id).home_object.is_some() {
                let m = self.mark();
                let t = self.temp();
                self.b.star(t);
                self.runtime_with(RuntimeId::SetHomeObject, &[t, obj]);
                self.b.ldar(t);
                self.release(m);
            }
            Ok(())
        } else {
            self.named_value(&p.value, name)
        }
    }

    fn object_property(&mut self, p: &'a oxc_ast::ast::ObjectProperty<'a>, obj: Register) -> Gen {
        let m = self.mark();
        let name = literal::property_name(&p.key);
        // `__proto__: v` sets the prototype.
        if p.kind == PropertyKind::Init
            && !p.computed
            && !p.method
            && !p.shorthand
            && matches!(&name, Some(PropertyName::Static(k)) if literal::is_proto_key(k))
        {
            let v = self.expr_to_temp(&p.value)?;
            self.runtime_with(RuntimeId::SetPrototypeInLiteral, &[obj, v]);
            self.release(m);
            return Ok(());
        }
        match p.kind {
            PropertyKind::Init => match name {
                Some(PropertyName::Static(key)) => {
                    self.object_value(p, obj, Some(key.clone()))?;
                    let k = self.b.string_constant(key);
                    let s = self.slot(FeedbackSlotKind::DefineNamedOwn);
                    self.b.define_named_own_property(obj, k, s);
                }
                _ => {
                    let key = self.property_key_to_reg(&p.key)?;
                    let anonymous = if p.method {
                        self.object_value(p, obj, None)?;
                        true
                    } else {
                        self.keyed_named_value(&p.value, key)?
                    };
                    let s = self.slot(FeedbackSlotKind::DefineKeyedOwn);
                    self.b
                        .define_keyed_own_property_in_literal(obj, key, u8::from(anonymous), s);
                }
            },
            PropertyKind::Get | PropertyKind::Set => {
                let args = self.temps(4);
                self.b.mov(obj, args.get(0));
                let fn_name = match &name {
                    Some(PropertyName::Static(key)) => {
                        let k = self.b.string_constant(key.clone());
                        self.b.lda_constant(k).star(args.get(1));
                        let prefix = if p.kind == PropertyKind::Get {
                            "get "
                        } else {
                            "set "
                        };
                        Some(literal::prefixed(prefix, key))
                    }
                    _ => {
                        self.property_key_into(&p.key, args.get(1))?;
                        None
                    }
                };
                let (fn_reg, other) = if p.kind == PropertyKind::Get {
                    (args.get(2), args.get(3))
                } else {
                    (args.get(3), args.get(2))
                };
                self.object_value(p, obj, fn_name)?;
                self.b.star(fn_reg).lda_undefined().star(other);
                self.runtime(RuntimeId::DefineAccessorInLiteral, args);
            }
        }
        self.release(m);
        Ok(())
    }

    /// A computed (or numeric) property key: `ToPropertyKey(key)` in a new temporary.
    pub(crate) fn property_key_to_reg(&mut self, key: &'a PropertyKey<'a>) -> Gen<Register> {
        let r = self.temp();
        self.property_key_into(key, r)?;
        Ok(r)
    }

    pub(crate) fn property_key_into(&mut self, key: &'a PropertyKey<'a>, r: Register) -> Gen {
        match literal::property_name(key) {
            Some(PropertyName::Static(name)) => {
                let k = self.b.string_constant(name);
                self.b.lda_constant(k).star(r);
            }
            _ => {
                let Some(e) = key.as_expression() else {
                    if let PropertyKey::PrivateIdentifier(p) = key {
                        return self.private_key_into(p, r);
                    }
                    return self.unsupported("private names", key.span());
                };
                self.expr(e)?;
                self.expr_pos(e.span().start);
                self.b.to_name().star(r);
            }
        }
        Ok(())
    }

    // ---- Member access ----

    fn super_parts(&mut self, sup: Span) -> Gen<(Register, Register)> {
        let this_ref = self.implicit(sup.start, ImplicitRef::This)?;
        let home_ref = self.implicit(sup.start, ImplicitRef::HomeObject)?;
        let this = self.temp();
        self.load_ref(this_ref, false);
        self.b.star(this);
        let home = self.temp();
        self.load_ref(home_ref, false);
        self.b.star(home);
        Ok((this, home))
    }

    /// Evaluate the object (and key) of a member expression used as an assignment target. The
    /// object is always copied: the right side may write a local (G4). Optional links jump to
    /// the chain end when the object is nullish.
    pub(crate) fn member_ref(&mut self, m: &'a MemberExpression<'a>) -> Gen<MemberRef> {
        match m {
            MemberExpression::StaticMemberExpression(s) => {
                let name = self.str_const(s.property.name.as_str());
                if let Expression::Super(sup) = &s.object {
                    let (this, home) = self.super_parts(sup.span)?;
                    return Ok(MemberRef::SuperNamed { this, home, name });
                }
                let obj = self.member_object(&s.object, None, s.optional, false)?;
                Ok(MemberRef::Named { obj, name })
            }
            MemberExpression::ComputedMemberExpression(c) => {
                if let Expression::Super(_) = &c.object {
                    return self.member_ref_of_computed(c);
                }
                let obj = self.member_object(&c.object, None, c.optional, false)?;
                let key = self.expr_to_temp(&c.expression)?;
                Ok(MemberRef::Keyed { obj, key })
            }
            MemberExpression::PrivateFieldExpression(p) => self.private_member_ref(p),
        }
    }

    /// The object of a member expression in a register. A plain local is used directly when
    /// `may_alias` and `later` (the key) cannot write it (G4).
    fn member_object(
        &mut self,
        object: &'a Expression<'a>,
        later: Option<&Expression<'_>>,
        optional: bool,
        may_alias: bool,
    ) -> Gen<Register> {
        if may_alias
            && !optional
            && later.is_none_or(write_free)
            && let Some(r) = self.local_register(object)
        {
            return Ok(r);
        }
        let r = self.temp();
        self.expr(object)?;
        if optional {
            self.optional_check()?;
        }
        self.b.star(r);
        Ok(r)
    }

    /// Jump to the end of the current optional chain if the accumulator is nullish.
    fn optional_check(&mut self) -> Gen {
        let Some(end) = self.chain_end else {
            return Err(super::internal("optional link outside a chain"));
        };
        self.b.jump_if_undefined_or_null(end);
        Ok(())
    }

    /// `acc = value of the evaluated member reference`.
    pub(crate) fn load_member(&mut self, r: MemberRef, at: u32) {
        match r {
            MemberRef::Named { obj, name } => {
                let s = self.slot(FeedbackSlotKind::LoadNamed);
                self.expr_pos(at);
                self.b.get_named_property(obj, name, s);
            }
            MemberRef::Keyed { obj, key } => {
                let s = self.slot(FeedbackSlotKind::LoadKeyed);
                self.b.ldar(key);
                self.expr_pos(at);
                self.b.get_keyed_property(obj, s);
            }
            MemberRef::SuperNamed { this, home, name } => {
                let s = self.slot(FeedbackSlotKind::LoadNamed);
                self.b.ldar(home);
                self.expr_pos(at);
                self.b.get_named_property_from_super(this, name, s);
            }
            MemberRef::SuperKeyed { this, home, key } => {
                self.runtime_with_at(RuntimeId::LoadKeyedFromSuper, &[this, home, key], Some(at));
            }
            MemberRef::PrivateBranded {
                obj,
                brand,
                get,
                set,
                at,
                ..
            } => self.private_branded_load(obj, brand, get, set.is_none(), at),
        }
    }

    /// PutValue of the accumulator into the member reference. Keeps the value in the
    /// accumulator when `keep`.
    pub(crate) fn store_member(&mut self, r: MemberRef, at: u32, keep: bool) {
        let m = self.mark();
        let saved = keep.then(|| {
            let t = self.temp();
            self.b.star(t);
            t
        });
        match r {
            MemberRef::Named { obj, name } => {
                let s = self.slot(FeedbackSlotKind::StoreNamed);
                self.expr_pos(at);
                self.b.set_named_property(obj, name, s);
            }
            MemberRef::Keyed { obj, key } => {
                let s = self.slot(FeedbackSlotKind::StoreKeyed);
                self.expr_pos(at);
                self.b.set_keyed_property(obj, key, s);
            }
            MemberRef::SuperNamed { this, home, name } => {
                let args = self.temps(4);
                self.b.star(args.get(3));
                self.b.mov(this, args.get(0)).mov(home, args.get(1));
                self.b.lda_constant(name).star(args.get(2));
                self.expr_pos(at);
                self.runtime(RuntimeId::StoreToSuper, args);
            }
            MemberRef::SuperKeyed { this, home, key } => {
                let args = self.temps(4);
                self.b.star(args.get(3));
                self.b
                    .mov(this, args.get(0))
                    .mov(home, args.get(1))
                    .mov(key, args.get(2));
                self.expr_pos(at);
                self.runtime(RuntimeId::StoreToSuper, args);
            }
            MemberRef::PrivateBranded {
                obj,
                brand,
                set,
                name,
                at,
                ..
            } => self.private_branded_store(obj, brand, set, name, at),
        }
        if let Some(t) = saved {
            self.b.ldar(t);
        }
        self.release(m);
    }

    fn static_member(&mut self, s: &'a StaticMemberExpression<'a>) -> Gen {
        let m = self.mark();
        let name = self.str_const(s.property.name.as_str());
        if let Expression::Super(sup) = &s.object {
            let (this, home) = self.super_parts(sup.span)?;
            self.load_member(
                MemberRef::SuperNamed { this, home, name },
                s.property.span.start,
            );
        } else {
            let obj = self.member_object(&s.object, None, s.optional, true)?;
            self.load_member(MemberRef::Named { obj, name }, s.property.span.start);
        }
        self.release(m);
        Ok(())
    }

    fn computed_member(&mut self, c: &'a ComputedMemberExpression<'a>) -> Gen {
        let m = self.mark();
        if let Expression::Super(_) = &c.object {
            let r = self.member_ref_of_computed(c)?;
            self.load_member(r, c.expression.span().start);
        } else {
            let obj = self.member_object(&c.object, Some(&c.expression), c.optional, true)?;
            self.expr(&c.expression)?;
            let s = self.slot(FeedbackSlotKind::LoadKeyed);
            self.expr_pos(c.expression.span().start);
            self.b.get_keyed_property(obj, s);
        }
        self.release(m);
        Ok(())
    }

    fn member_ref_of_computed(&mut self, c: &'a ComputedMemberExpression<'a>) -> Gen<MemberRef> {
        let Expression::Super(sup) = &c.object else {
            unreachable!("super member")
        };
        let this_ref = self.implicit(sup.span.start, ImplicitRef::This)?;
        let this = self.temp();
        self.load_ref(this_ref, false);
        self.b.star(this);
        // The reference keeps the key value: GetValue and PutValue convert it (ECMA-262
        // SuperProperty evaluation), so `super[k] = v` evaluates `v` before ToPropertyKey(k).
        // LoadKeyedFromSuper and StoreToSuper convert it.
        let key = self.temp();
        self.expr(&c.expression)?;
        self.b.star(key);
        let home_ref = self.implicit(sup.span.start, ImplicitRef::HomeObject)?;
        let home = self.temp();
        self.load_ref(home_ref, false);
        self.b.star(home);
        Ok(MemberRef::SuperKeyed { this, home, key })
    }

    fn chain(&mut self, c: &'a ChainExpression<'a>) -> Gen {
        let nullish = self.b.new_label();
        let saved = self.chain_end.replace(nullish);
        let result = match &c.expression {
            ChainElement::CallExpression(call) => self.call(call),
            ChainElement::TSNonNullExpression(t) => self.expr(&t.expression),
            other => {
                let m = other.to_member_expression();
                match m {
                    MemberExpression::StaticMemberExpression(s) => self.static_member(s),
                    MemberExpression::ComputedMemberExpression(cm) => self.computed_member(cm),
                    MemberExpression::PrivateFieldExpression(p) => self.private_get(p),
                }
            }
        };
        self.chain_end = saved;
        result?;
        let done = self.b.new_label();
        self.b.jump(done);
        self.b.bind(nullish);
        self.b.lda_undefined();
        self.b.bind(done);
        Ok(())
    }

    // ---- Calls ----

    /// Evaluate the callee into `callee`. For a property call returns a list of `1 + n`
    /// registers whose first one holds the receiver.
    fn callee_and_receiver(
        &mut self,
        callee_expr: &'a Expression<'a>,
        callee: Register,
        n: u32,
    ) -> Gen<Option<RegisterList>> {
        let inner = callee_expr.without_parentheses();
        // `(a?.b)()`: an optional chain that ends in a property access is still a reference,
        // so the call gets its receiver. A nullish base gives an undefined callee (the call
        // throws a TypeError).
        if let Expression::ChainExpression(chain) = inner
            && let Some(member) = chain.expression.as_member_expression()
        {
            let nullish = self.b.new_label();
            let saved = self.chain_end.replace(nullish);
            let list = self.member_callee(member, callee, n);
            self.chain_end = saved;
            let list = list?;
            let done = self.b.new_label();
            self.b.jump(done);
            self.b.bind(nullish);
            self.b.lda_undefined().star(callee);
            self.b.bind(done);
            return Ok(Some(list));
        }
        let member = match inner {
            Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
            | Expression::PrivateFieldExpression(_) => inner.as_member_expression(),
            _ => None,
        };
        let Some(member) = member else {
            // A dynamic lookup (`with`, sloppy direct eval): lookup.rs.
            if let Expression::Identifier(id) = inner
                && matches!(
                    self.reference_of(id)?.resolution,
                    Resolution::Dynamic { .. }
                )
            {
                return self.dynamic_callee(callee_expr, id, callee, n);
            }
            self.expr(callee_expr)?;
            self.b.star(callee);
            return Ok(None);
        };
        self.member_callee(member, callee, n).map(Some)
    }

    /// Evaluate the callee of a property call into `callee`. Returns a list of `1 + n`
    /// registers whose first one holds the receiver.
    fn member_callee(
        &mut self,
        member: &'a MemberExpression<'a>,
        callee: Register,
        n: u32,
    ) -> Gen<RegisterList> {
        let list = self.temps(1 + n);
        match member {
            MemberExpression::StaticMemberExpression(s) => {
                let name = self.str_const(s.property.name.as_str());
                if let Expression::Super(sup) = &s.object {
                    let m = self.mark();
                    let (this, home) = self.super_parts(sup.span)?;
                    self.load_member(
                        MemberRef::SuperNamed { this, home, name },
                        s.property.span.start,
                    );
                    self.b.star(callee).mov(this, list.get(0));
                    self.release(m);
                } else {
                    self.expr(&s.object)?;
                    if s.optional {
                        self.optional_check()?;
                    }
                    self.b.star(list.get(0));
                    self.load_member(
                        MemberRef::Named {
                            obj: list.get(0),
                            name,
                        },
                        s.property.span.start,
                    );
                    self.b.star(callee);
                }
            }
            MemberExpression::ComputedMemberExpression(c) => {
                if let Expression::Super(_) = &c.object {
                    let m = self.mark();
                    let r = self.member_ref_of_computed(c)?;
                    self.load_member(r, c.expression.span().start);
                    self.b.star(callee);
                    if let MemberRef::SuperKeyed { this, .. } = r {
                        self.b.mov(this, list.get(0));
                    }
                    self.release(m);
                } else {
                    self.expr(&c.object)?;
                    if c.optional {
                        self.optional_check()?;
                    }
                    self.b.star(list.get(0));
                    self.expr(&c.expression)?;
                    let s = self.slot(FeedbackSlotKind::LoadKeyed);
                    self.expr_pos(c.expression.span().start);
                    self.b.get_keyed_property(list.get(0), s).star(callee);
                }
            }
            MemberExpression::PrivateFieldExpression(p) => {
                self.private_callee(p, callee, list)?;
            }
        }
        Ok(list)
    }

    /// Emit the call: `args` holds the receiver first when `property`.
    fn emit_call(&mut self, callee: Register, args: RegisterList, property: bool, spread: bool) {
        let s = self.slot(FeedbackSlotKind::Call);
        if spread {
            self.b.call_with_spread(callee, args, args.len(), s);
            return;
        }
        if property {
            match args.len() {
                1 => self.b.call_property0(callee, args.get(0), s),
                2 => self.b.call_property1(callee, args.get(0), args.get(1), s),
                3 => self
                    .b
                    .call_property2(callee, args.get(0), args.get(1), args.get(2), s),
                n => self.b.call_property(callee, args, n, s),
            };
        } else {
            match args.len() {
                0 => self.b.call_undefined_receiver0(callee, s),
                1 => self.b.call_undefined_receiver1(callee, args.get(0), s),
                2 => self
                    .b
                    .call_undefined_receiver2(callee, args.get(0), args.get(1), s),
                n => self.b.call_undefined_receiver(callee, args, n, s),
            };
        }
    }

    /// Spread arguments are simple when only the last one is a spread.
    fn spread_kind(args: &[Argument<'_>]) -> (bool, bool) {
        let any = args.iter().any(|a| matches!(a, Argument::SpreadElement(_)));
        let last_only = any
            && args[..args.len() - 1]
                .iter()
                .all(|a| !matches!(a, Argument::SpreadElement(_)));
        (any, last_only)
    }

    /// Evaluate arguments into `list` from index `first`. A spread argument (only the last
    /// one comes here) becomes a new Array of its values at once: ArgumentListEvaluation
    /// (§13.3.8.1) runs GetIterator and the steps while the arguments are evaluated. The
    /// spread operand of CallWithSpread and ConstructWithSpread is always an Array that
    /// bytecode made (this one, the array of `arguments_array`, or the rest array of a
    /// default derived constructor), and the VM spreads it as it is, with no `@@iterator`
    /// call: a second iteration would be observable.
    fn arguments_into(&mut self, args: &'a [Argument<'a>], list: RegisterList, first: u32) -> Gen {
        for (i, a) in args.iter().enumerate() {
            let r = list.get(first + i as u32);
            match a {
                Argument::SpreadElement(s) => {
                    self.expr(&s.argument)?;
                    self.expr_pos(s.span.start);
                    self.b.create_array_from_iterable().star(r);
                }
                other => self.expr_into(other.to_expression(), r)?,
            }
        }
        Ok(())
    }

    /// All arguments as one array (for a spread that is not last): `acc = [...args]`.
    fn arguments_array(&mut self, args: &'a [Argument<'a>], at: u32) -> Gen {
        let m = self.mark();
        let arr = self.temp();
        let idx = self.temp();
        let s = self.slot(FeedbackSlotKind::Literal);
        self.b.create_empty_array_literal(s).star(arr);
        self.b.lda_zero().star(idx);
        let store = self.slot(FeedbackSlotKind::StoreInArrayLiteral);
        let inc = self.slot(FeedbackSlotKind::BinaryOp);
        for a in args {
            match a {
                Argument::SpreadElement(sp) => {
                    self.expr(&sp.argument)?;
                    self.spread_into_array(arr, idx, sp.span.start);
                }
                other => {
                    self.expr(other.to_expression())?;
                    self.b.sta_in_array_literal(arr, idx, store);
                    self.b.ldar(idx).inc(inc).star(idx);
                }
            }
        }
        let _ = at;
        self.b.ldar(arr);
        self.release(m);
        Ok(())
    }

    fn call(&mut self, c: &'a CallExpression<'a>) -> Gen {
        if let Expression::Super(sup) = &c.callee {
            return self.super_call(c, sup.span);
        }
        if is_direct_eval(c) {
            return self.eval_call(c);
        }
        let m = self.mark();
        let callee = self.temp();
        let args = &c.arguments;
        let (any_spread, last_only) = Self::spread_kind(args);
        let n = if any_spread && !last_only {
            1
        } else {
            args.len() as u32
        };
        let receiver = self.callee_and_receiver(&c.callee, callee, n)?;
        if c.optional {
            self.b.ldar(callee);
            self.optional_check()?;
        }
        let at = call_position(c);
        match receiver {
            Some(list) => {
                if any_spread && !last_only {
                    self.arguments_array(args, at)?;
                    self.b.star(list.get(1));
                } else {
                    self.arguments_into(args, list, 1)?;
                }
                self.expr_pos(at);
                self.emit_call(callee, list, true, any_spread);
            }
            None => {
                if any_spread {
                    // CallWithSpread takes the receiver first.
                    let list = self.temps(1 + n);
                    self.b.lda_undefined().star(list.get(0));
                    if last_only {
                        self.arguments_into(args, list, 1)?;
                    } else {
                        self.arguments_array(args, at)?;
                        self.b.star(list.get(1));
                    }
                    self.expr_pos(at);
                    self.emit_call(callee, list, true, true);
                } else {
                    let list = self.temps(n);
                    self.arguments_into(args, list, 0)?;
                    self.expr_pos(at);
                    self.emit_call(callee, list, false, false);
                }
            }
        }
        self.release(m);
        Ok(())
    }

    fn eval_call(&mut self, c: &'a CallExpression<'a>) -> Gen {
        let (any_spread, _) = Self::spread_kind(&c.arguments);
        if any_spread {
            return self.unsupported("direct eval with spread arguments", c.span);
        }
        let m = self.mark();
        let callee = self.temp();
        let n = c.arguments.len() as u32;
        // A Dynamic `eval` (inside `with` or a sloppy-eval scope) is loaded with its receiver
        // (LoadLookupSlotForCall, M2-P2-L5-OPEN): a `with` object's own `eval` that is not
        // %eval% is then called as a method of that object (§13.3.6.1 step 4, WithBaseObject).
        // Direct eval code does not read its receiver, so the closure from
        // ResolvePossiblyDirectEval gets the same call.
        let dynamic = match c.callee.without_parentheses() {
            Expression::Identifier(id)
                if matches!(
                    self.reference_of(id)?.resolution,
                    Resolution::Dynamic { .. }
                ) =>
            {
                Some(id)
            }
            _ => None,
        };
        let (list, first) = match dynamic {
            Some(id) => {
                let list = self.dynamic_callee(&c.callee, id, callee, n)?;
                (list.expect("dynamic_callee gives a receiver list"), 1)
            }
            None => {
                self.expr(&c.callee)?;
                self.b.star(callee);
                (self.temps(n), 0)
            }
        };
        self.arguments_into(&c.arguments, list, first)?;
        let resolve = self.temps(5);
        self.b.mov(callee, resolve.get(0));
        if n > 0 {
            self.b.mov(list.get(first), resolve.get(1));
        } else {
            self.b.lda_undefined().star(resolve.get(1));
        }
        self.b.lda_current_function().star(resolve.get(2));
        let scope = self.cx.context_scope(self.scope);
        self.b.lda_smi(scope.0 as i32).star(resolve.get(3));
        self.b.lda_smi(c.span.start as i32).star(resolve.get(4));
        self.expr_pos(c.span.start);
        self.runtime(RuntimeId::ResolvePossiblyDirectEval, resolve);
        self.b.star(callee);
        self.emit_call(callee, list, first == 1, false);
        self.release(m);
        Ok(())
    }

    fn super_call(&mut self, c: &'a CallExpression<'a>, sup: Span) -> Gen {
        let callee_ref = self.implicit(sup.start, ImplicitRef::Callee)?;
        let nt_ref = self.implicit(sup.start, ImplicitRef::NewTarget)?;
        let this_ref = self.implicit(sup.start, ImplicitRef::This)?;
        let m = self.mark();
        let ctor = self.temp();
        let result = self.temp();
        self.load_ref(callee_ref, false);
        self.b.get_super_constructor(ctor);
        let args = &c.arguments;
        let (any_spread, last_only) = Self::spread_kind(args);
        let n = if any_spread && !last_only {
            1
        } else {
            args.len() as u32
        };
        let list = self.temps(n);
        if any_spread && !last_only {
            self.arguments_array(args, c.span.start)?;
            self.b.star(list.get(0));
        } else {
            self.arguments_into(args, list, 0)?;
        }
        self.expr_pos(sup.start);
        self.b.throw_if_not_super_constructor(ctor);
        self.load_ref(nt_ref, false);
        let s = self.slot(FeedbackSlotKind::Call);
        self.expr_pos(c.span.start);
        if any_spread {
            self.b.construct_with_spread(ctor, list, n, s);
        } else {
            self.b.construct(ctor, list, n, s);
        }
        self.b.star(result);
        match this_ref.resolution {
            Resolution::Binding { binding, hops, .. } => {
                // Bind `this`: it must still be the hole.
                self.load_binding_raw(binding, hops, false);
                self.b.throw_super_already_called_if_not_hole();
                self.b.ldar(result);
                self.store_binding_raw(binding, hops);
                self.fields_after_super(callee_ref, result)?;
            }
            Resolution::Dynamic { .. } => self.eval_bind_this(this_ref, callee_ref, result),
            Resolution::Global => return Err(super::internal("super() without a this binding")),
        }
        self.release(m);
        Ok(())
    }

    /// `super()` in direct eval code (E6, M3-P4): the caller's `this`, `.callee` and
    /// `new.target` are context slots of the derived constructor, reached by name.
    /// BindThisValue (§13.3.7.1 step 8) is `StaLookupSlot "this"`: no source text can assign
    /// to `this`, so the VM gives a store to that name the BindThisValue meaning (a
    /// ReferenceError if `this` is already bound; `slow/lookup.rs`). Then
    /// InitializeInstanceElements(result, F) with F = `.callee`: whether F's class has an
    /// instance initializer is not known here, and `RunClassFieldsInitializer` does nothing
    /// without one. The accumulator holds `this` afterwards.
    fn eval_bind_this(&mut self, this_ref: &Reference, callee_ref: &Reference, result: Register) {
        let k = self.name_const(this_ref.name);
        let flags = u8::from(self.strict_at(this_ref));
        self.b.ldar(result);
        self.expr_pos(this_ref.span.start);
        self.b.sta_lookup_slot(k, flags);
        let m = self.mark();
        let args = self.temps(2);
        self.load_ref(callee_ref, false);
        self.b.star(args.get(0)).mov(result, args.get(1));
        self.expr_pos(callee_ref.span.start);
        self.runtime(RuntimeId::RunClassFieldsInitializer, args);
        self.release(m);
        self.b.ldar(result);
    }

    fn new_expr(&mut self, n: &'a NewExpression<'a>) -> Gen {
        let m = self.mark();
        let ctor = self.expr_to_temp(&n.callee)?;
        let args = &n.arguments;
        let (any_spread, last_only) = Self::spread_kind(args);
        let count = if any_spread && !last_only {
            1
        } else {
            args.len() as u32
        };
        let list = self.temps(count);
        if any_spread && !last_only {
            self.arguments_array(args, n.span.start)?;
            self.b.star(list.get(0));
        } else {
            self.arguments_into(args, list, 0)?;
        }
        let s = self.slot(FeedbackSlotKind::Call);
        self.b.ldar(ctor);
        self.expr_pos(n.span.start);
        if any_spread {
            self.b.construct_with_spread(ctor, list, count, s);
        } else {
            self.b.construct(ctor, list, count, s);
        }
        self.release(m);
        Ok(())
    }

    // ---- Operators ----

    fn emit_arith(&mut self, op: Arith, lhs: Register) {
        let s = self.slot(FeedbackSlotKind::BinaryOp);
        match op {
            Arith::Add => self.b.add(lhs, s),
            Arith::Sub => self.b.sub(lhs, s),
            Arith::Mul => self.b.mul(lhs, s),
            Arith::Div => self.b.div(lhs, s),
            Arith::Mod => self.b.r#mod(lhs, s),
            Arith::Exp => self.b.exp(lhs, s),
            Arith::BitOr => self.b.bitwise_or(lhs, s),
            Arith::BitXor => self.b.bitwise_xor(lhs, s),
            Arith::BitAnd => self.b.bitwise_and(lhs, s),
            Arith::Shl => self.b.shift_left(lhs, s),
            Arith::Sar => self.b.shift_right(lhs, s),
            Arith::Shr => self.b.shift_right_logical(lhs, s),
        };
    }

    fn emit_arith_smi(&mut self, op: Arith, imm: i32) {
        let s = self.slot(FeedbackSlotKind::BinaryOp);
        match op {
            Arith::Add => self.b.add_smi(imm, s),
            Arith::Sub => self.b.sub_smi(imm, s),
            Arith::Mul => self.b.mul_smi(imm, s),
            Arith::Div => self.b.div_smi(imm, s),
            Arith::Mod => self.b.mod_smi(imm, s),
            Arith::Exp => self.b.exp_smi(imm, s),
            Arith::BitOr => self.b.bitwise_or_smi(imm, s),
            Arith::BitXor => self.b.bitwise_xor_smi(imm, s),
            Arith::BitAnd => self.b.bitwise_and_smi(imm, s),
            Arith::Shl => self.b.shift_left_smi(imm, s),
            Arith::Sar => self.b.shift_right_smi(imm, s),
            Arith::Shr => self.b.shift_right_logical_smi(imm, s),
        };
    }

    /// `acc = lhs OP acc` for a compare operator. Returns false if `op` is not one.
    fn emit_compare(&mut self, op: BinaryOperator, lhs: Register) -> bool {
        match op {
            BinaryOperator::Equality | BinaryOperator::Inequality => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_equal(lhs, s);
            }
            BinaryOperator::StrictEquality | BinaryOperator::StrictInequality => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_equal_strict(lhs, s);
            }
            BinaryOperator::LessThan => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_less_than(lhs, s);
            }
            BinaryOperator::LessEqualThan => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_less_than_or_equal(lhs, s);
            }
            BinaryOperator::GreaterThan => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_greater_than(lhs, s);
            }
            BinaryOperator::GreaterEqualThan => {
                let s = self.slot(FeedbackSlotKind::CompareOp);
                self.b.test_greater_than_or_equal(lhs, s);
            }
            BinaryOperator::In => {
                let s = self.slot(FeedbackSlotKind::HasKeyed);
                self.b.test_in(lhs, s);
            }
            BinaryOperator::Instanceof => {
                let s = self.slot(FeedbackSlotKind::InstanceOf);
                self.b.test_instance_of(lhs, s);
            }
            _ => return false,
        }
        if matches!(
            op,
            BinaryOperator::Inequality | BinaryOperator::StrictInequality
        ) {
            self.b.logical_not();
        }
        true
    }

    /// `typeof x === "literal"` and `x === null` forms.
    fn special_compare(&mut self, b: &'a BinaryExpression<'a>) -> Gen<bool> {
        let negate = match b.operator {
            BinaryOperator::StrictEquality => false,
            BinaryOperator::StrictInequality => true,
            BinaryOperator::Equality | BinaryOperator::Inequality => {
                // `typeof x == "s"` is the same as `===`.
                if !matches!(b.left.without_parentheses(), Expression::UnaryExpression(u) if u.operator == UnaryOperator::Typeof)
                {
                    return Ok(false);
                }
                b.operator == BinaryOperator::Inequality
            }
            _ => return Ok(false),
        };
        if let (Expression::UnaryExpression(u), Expression::StringLiteral(s)) =
            (b.left.without_parentheses(), b.right.without_parentheses())
            && u.operator == UnaryOperator::Typeof
        {
            let lit = match s.value.as_str() {
                Some("number") => TypeOfLiteral::Number,
                Some("string") => TypeOfLiteral::String,
                Some("symbol") => TypeOfLiteral::Symbol,
                Some("boolean") => TypeOfLiteral::Boolean,
                Some("bigint") => TypeOfLiteral::BigInt,
                Some("undefined") => TypeOfLiteral::Undefined,
                Some("function") => TypeOfLiteral::Function,
                Some("object") => TypeOfLiteral::Object,
                _ => TypeOfLiteral::Other,
            };
            self.typeof_operand(&u.argument)?;
            self.b.test_type_of(lit);
            if negate {
                self.b.logical_not();
            }
            return Ok(true);
        }
        if matches!(
            b.operator,
            BinaryOperator::StrictEquality | BinaryOperator::StrictInequality
        ) && matches!(b.right.without_parentheses(), Expression::NullLiteral(_))
        {
            self.expr(&b.left)?;
            self.b.test_null();
            if negate {
                self.b.logical_not();
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn binary(&mut self, b: &'a BinaryExpression<'a>) -> Gen {
        if self.special_compare(b)? {
            return Ok(());
        }
        let at = b.span.start;
        if let Some(op) = arith_of(b.operator)
            && let Some(imm) = smi_literal(&b.right)
        {
            self.expr(&b.left)?;
            self.expr_pos(at);
            self.emit_arith_smi(op, imm);
            return Ok(());
        }
        let m = self.mark();
        let lhs = self.operand_reg(&b.left, &b.right)?;
        self.expr(&b.right)?;
        self.expr_pos(at);
        if let Some(op) = arith_of(b.operator) {
            self.emit_arith(op, lhs);
        } else if !self.emit_compare(b.operator, lhs) {
            return Err(super::internal(format!("binary operator {:?}", b.operator)));
        }
        self.release(m);
        Ok(())
    }

    fn logical(&mut self, l: &'a LogicalExpression<'a>) -> Gen {
        let end = self.b.new_label();
        self.expr(&l.left)?;
        match l.operator {
            LogicalOperator::And => {
                self.b.jump_if_to_boolean_false(end);
            }
            LogicalOperator::Or => {
                self.b.jump_if_to_boolean_true(end);
            }
            LogicalOperator::Coalesce => {
                let rhs = self.b.new_label();
                self.b.jump_if_undefined_or_null(rhs);
                self.b.jump(end);
                self.b.bind(rhs);
            }
        }
        self.expr(&l.right)?;
        self.b.bind(end);
        Ok(())
    }

    fn conditional(&mut self, c: &'a ConditionalExpression<'a>) -> Gen {
        let (else_label, end) = (self.b.new_label(), self.b.new_label());
        self.branch_if_false(&c.test, else_label)?;
        self.expr(&c.consequent)?;
        self.b.jump(end);
        self.b.bind(else_label);
        self.expr(&c.alternate)?;
        self.b.bind(end);
        Ok(())
    }

    /// Jump to `target` if `e` is falsy (V8 VisitForTest, simplified).
    pub(crate) fn branch_if_false(&mut self, e: &'a Expression<'a>, target: Label) -> Gen {
        self.enter(e.span())?;
        let result = self.branch(e, target, false);
        self.leave();
        result
    }

    pub(crate) fn branch_if_true(&mut self, e: &'a Expression<'a>, target: Label) -> Gen {
        self.enter(e.span())?;
        let result = self.branch(e, target, true);
        self.leave();
        result
    }

    fn branch(&mut self, e: &'a Expression<'a>, target: Label, when: bool) -> Gen {
        match e {
            Expression::ParenthesizedExpression(p) => self.branch(&p.expression, target, when),
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
                if when {
                    self.branch_if_false(&u.argument, target)
                } else {
                    self.branch_if_true(&u.argument, target)
                }
            }
            Expression::LogicalExpression(l) if l.operator != LogicalOperator::Coalesce => {
                let and = l.operator == LogicalOperator::And;
                if and != when {
                    // (a && b) false -> a false or b false; (a || b) true -> a true or b true.
                    self.branch_dir(&l.left, target, when)?;
                    self.branch_dir(&l.right, target, when)
                } else {
                    let skip = self.b.new_label();
                    self.branch_dir(&l.left, skip, !when)?;
                    self.branch_dir(&l.right, target, when)?;
                    self.b.bind(skip);
                    Ok(())
                }
            }
            Expression::BooleanLiteral(b) => {
                if b.value == when {
                    self.b.jump(target);
                }
                Ok(())
            }
            Expression::BinaryExpression(b)
                if b.operator.is_equality()
                    || b.operator.is_compare()
                    || matches!(b.operator, BinaryOperator::In | BinaryOperator::Instanceof) =>
            {
                self.binary(b)?;
                if when {
                    self.b.jump_if_true(target);
                } else {
                    self.b.jump_if_false(target);
                }
                Ok(())
            }
            _ => {
                self.expr(e)?;
                if when {
                    self.b.jump_if_to_boolean_true(target);
                } else {
                    self.b.jump_if_to_boolean_false(target);
                }
                Ok(())
            }
        }
    }

    fn branch_dir(&mut self, e: &'a Expression<'a>, target: Label, when: bool) -> Gen {
        if when {
            self.branch_if_true(e, target)
        } else {
            self.branch_if_false(e, target)
        }
    }

    /// The operand of `typeof`: an unresolvable name gives undefined, not a ReferenceError.
    fn typeof_operand(&mut self, e: &'a Expression<'a>) -> Gen {
        match e.without_parentheses() {
            Expression::Identifier(id) => {
                let r = self.reference_of(id)?;
                self.load_ref(r, true);
                Ok(())
            }
            _ => self.expr(e),
        }
    }

    fn unary(&mut self, u: &'a UnaryExpression<'a>) -> Gen {
        match u.operator {
            UnaryOperator::Typeof => {
                self.typeof_operand(&u.argument)?;
                let s = self.slot(FeedbackSlotKind::TypeOf);
                self.b.type_of(s);
            }
            UnaryOperator::Void => {
                self.expr_effect(&u.argument)?;
                self.b.lda_undefined();
            }
            UnaryOperator::LogicalNot => {
                self.expr(&u.argument)?;
                self.b.to_boolean_logical_not();
            }
            UnaryOperator::UnaryNegation => {
                if let Expression::NumericLiteral(n) = &u.argument {
                    self.number(-n.value);
                    return Ok(());
                }
                self.expr(&u.argument)?;
                let s = self.slot(FeedbackSlotKind::BinaryOp);
                self.expr_pos(u.span.start);
                self.b.negate(s);
            }
            UnaryOperator::UnaryPlus => {
                self.expr(&u.argument)?;
                let s = self.slot(FeedbackSlotKind::BinaryOp);
                self.expr_pos(u.span.start);
                self.b.to_number(s);
            }
            UnaryOperator::BitwiseNot => {
                self.expr(&u.argument)?;
                let s = self.slot(FeedbackSlotKind::BinaryOp);
                self.expr_pos(u.span.start);
                self.b.bitwise_not(s);
            }
            UnaryOperator::Delete => self.delete(u)?,
        }
        Ok(())
    }

    fn delete(&mut self, u: &'a UnaryExpression<'a>) -> Gen {
        let strict = self.strict();
        let arg = u.argument.without_parentheses();
        match arg {
            Expression::StaticMemberExpression(_) | Expression::ComputedMemberExpression(_) => {
                self.delete_member(arg.as_member_expression().expect("member"), u.span, strict)
            }
            Expression::ChainExpression(c) => {
                let nullish = self.b.new_label();
                let saved = self.chain_end.replace(nullish);
                let result = match &c.expression {
                    ChainElement::StaticMemberExpression(_)
                    | ChainElement::ComputedMemberExpression(_) => {
                        self.delete_member(c.expression.to_member_expression(), u.span, strict)
                    }
                    _ => self.expr(arg).map(|()| {
                        self.b.lda_true();
                    }),
                };
                self.chain_end = saved;
                result?;
                let done = self.b.new_label();
                self.b.jump(done);
                self.b.bind(nullish);
                self.b.lda_true();
                self.b.bind(done);
                Ok(())
            }
            Expression::Identifier(id) => {
                let r = self.reference_of(id)?;
                match r.resolution {
                    Resolution::Binding { .. } => {
                        self.b.lda_false();
                    }
                    Resolution::Global | Resolution::Dynamic { .. } => {
                        let m = self.mark();
                        let name = self.temp();
                        let k = self.name_const(r.name);
                        self.b.lda_constant(k).star(name);
                        self.runtime_with_at(
                            RuntimeId::DeleteLookupSlot,
                            &[name],
                            Some(u.span.start),
                        );
                        self.release(m);
                    }
                }
                Ok(())
            }
            Expression::PrivateFieldExpression(p) => self.private_delete(p),
            _ => {
                self.expr_effect(&u.argument)?;
                self.b.lda_true();
                Ok(())
            }
        }
    }

    fn delete_member(&mut self, m: &'a MemberExpression<'a>, at: Span, strict: bool) -> Gen {
        if let Expression::Super(s) = m.object() {
            return self.unsupported("delete super", s.span);
        }
        let mark = self.mark();
        match m {
            MemberExpression::StaticMemberExpression(s) => {
                let obj = self.member_object(&s.object, None, s.optional, true)?;
                let k = self.str_const(s.property.name.as_str());
                self.b.lda_constant(k);
                self.expr_pos(at.start);
                if strict {
                    self.b.delete_property_strict(obj);
                } else {
                    self.b.delete_property_sloppy(obj);
                }
            }
            MemberExpression::ComputedMemberExpression(c) => {
                let obj = self.member_object(&c.object, Some(&c.expression), c.optional, true)?;
                self.expr(&c.expression)?;
                self.expr_pos(at.start);
                if strict {
                    self.b.delete_property_strict(obj);
                } else {
                    self.b.delete_property_sloppy(obj);
                }
            }
            MemberExpression::PrivateFieldExpression(p) => {
                self.private_delete(p)?;
            }
        }
        self.release(mark);
        Ok(())
    }

    // ---- Assignment and update ----

    /// Evaluate the reference parts of a simple assignment target.
    pub(crate) fn prepare_simple(
        &mut self,
        t: &'a SimpleAssignmentTarget<'a>,
    ) -> Gen<Prepared<'a, 's>> {
        match t {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                Ok(Prepared::Binding(self.reference_of(id)?))
            }
            SimpleAssignmentTarget::TSAsExpression(e) => self.prepare_expr_target(&e.expression),
            SimpleAssignmentTarget::TSSatisfiesExpression(e) => {
                self.prepare_expr_target(&e.expression)
            }
            SimpleAssignmentTarget::TSNonNullExpression(e) => {
                self.prepare_expr_target(&e.expression)
            }
            SimpleAssignmentTarget::TSTypeAssertion(e) => self.prepare_expr_target(&e.expression),
            other => {
                let m = other.to_member_expression();
                Ok(Prepared::Member(self.member_ref(m)?))
            }
        }
    }

    fn prepare_expr_target(&mut self, e: &'a Expression<'a>) -> Gen<Prepared<'a, 's>> {
        match e.without_parentheses() {
            Expression::Identifier(id) => Ok(Prepared::Binding(self.reference_of(id)?)),
            inner => match inner.as_member_expression() {
                Some(m) => Ok(Prepared::Member(self.member_ref(m)?)),
                None => self.unsupported("assignment target", e.span()),
            },
        }
    }

    pub(crate) fn prepare_target(&mut self, t: &'a AssignmentTarget<'a>) -> Gen<Prepared<'a, 's>> {
        match t.as_simple_assignment_target() {
            Some(s) => self.prepare_simple(s),
            None => Ok(Prepared::Pattern(t.to_assignment_target_pattern())),
        }
    }

    /// PutValue of the accumulator into a prepared target.
    pub(crate) fn put_prepared(&mut self, p: &Prepared<'a, 's>, at: u32, keep: bool) -> Gen {
        match p {
            Prepared::Binding(r) => {
                self.store_ref(r, keep);
                Ok(())
            }
            Prepared::Dynamic { r, regs } => {
                let strict = self.strict_at(r);
                self.store_resolved(r.span.start, *regs, strict);
                Ok(())
            }
            Prepared::Member(m) => {
                self.store_member(*m, at, keep);
                Ok(())
            }
            Prepared::Pattern(pat) => {
                let m = self.mark();
                let v = self.temp();
                self.b.star(v);
                self.destructure_assign(pat, v)?;
                if keep {
                    self.b.ldar(v);
                }
                self.release(m);
                Ok(())
            }
        }
    }

    /// `acc = current value of a prepared target`.
    fn get_prepared(&mut self, p: &Prepared<'a, 's>, at: u32) {
        match p {
            Prepared::Binding(r) => self.load_ref(r, false),
            Prepared::Dynamic { r, regs } => {
                let strict = self.strict_at(r);
                self.load_resolved(r.span.start, *regs, strict);
            }
            Prepared::Member(m) => {
                // GetValue stores the converted key in the reference, so the PutValue that
                // follows does not convert it again (ECMA-262 GetValue step 3.c).
                if let MemberRef::SuperKeyed { key, .. } = *m {
                    self.b.ldar(key).to_name().star(key);
                }
                self.load_member(*m, at);
            }
            Prepared::Pattern(_) => unreachable!("compound assignment to a pattern"),
        }
    }

    /// Assign `value` to a for-in/of head target or a destructuring target.
    pub(crate) fn assign_to_target(&mut self, t: &'a AssignmentTarget<'a>, value: Register) -> Gen {
        let m = self.mark();
        let p = self.prepare_target(t)?;
        self.b.ldar(value);
        self.put_prepared(&p, t.span().start, false)?;
        self.release(m);
        Ok(())
    }

    fn assignment(&mut self, a: &'a AssignmentExpression<'a>, keep: bool) -> Gen {
        let m = self.mark();
        let at = a.left.span().start;
        let name = match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                Some(UnitString::new(id.name.as_str()))
            }
            _ => None,
        };
        let target = self.prepare_target(&a.left)?;
        let target = if a.operator == AssignmentOperator::Assign && runs_no_code(&a.right) {
            target
        } else {
            self.resolve_dynamic(target)
        };
        match a.operator {
            AssignmentOperator::Assign => {
                self.named_value(&a.right, name)?;
                self.put_prepared(&target, at, keep)?;
            }
            AssignmentOperator::LogicalOr
            | AssignmentOperator::LogicalAnd
            | AssignmentOperator::LogicalNullish => {
                let end = self.b.new_label();
                self.get_prepared(&target, at);
                match a.operator {
                    AssignmentOperator::LogicalOr => {
                        self.b.jump_if_to_boolean_true(end);
                    }
                    AssignmentOperator::LogicalAnd => {
                        self.b.jump_if_to_boolean_false(end);
                    }
                    _ => {
                        let assign = self.b.new_label();
                        self.b.jump_if_undefined_or_null(assign);
                        self.b.jump(end);
                        self.b.bind(assign);
                    }
                }
                self.named_value(&a.right, name)?;
                self.put_prepared(&target, at, keep)?;
                self.b.bind(end);
            }
            op => {
                let Some(bin) = op.to_binary_operator() else {
                    return Err(super::internal(format!("assignment {op:?}")));
                };
                let arith = arith_of(bin).expect("compound assignment is arithmetic");
                self.get_prepared(&target, at);
                if let Some(imm) = smi_literal(&a.right) {
                    self.expr_pos(a.span.start);
                    self.emit_arith_smi(arith, imm);
                } else {
                    let old = self.temp();
                    self.b.star(old);
                    self.expr(&a.right)?;
                    self.expr_pos(a.span.start);
                    self.emit_arith(arith, old);
                }
                self.put_prepared(&target, at, keep)?;
            }
        }
        self.release(m);
        Ok(())
    }

    fn update(&mut self, u: &'a UpdateExpression<'a>, keep: bool) -> Gen {
        let m = self.mark();
        let at = u.argument.span().start;
        let target = self.prepare_simple(&u.argument)?;
        let target = self.resolve_dynamic(target);
        self.get_prepared(&target, at);
        let s = self.slot(FeedbackSlotKind::BinaryOp);
        let old = (keep && !u.prefix).then(|| {
            let r = self.temp();
            self.b.to_numeric(s).star(r);
            r
        });
        self.expr_pos(u.span.start);
        match u.operator {
            UpdateOperator::Increment => self.b.inc(s),
            UpdateOperator::Decrement => self.b.dec(s),
        };
        self.put_prepared(&target, at, keep && u.prefix)?;
        if let Some(r) = old {
            self.b.ldar(r);
        }
        self.release(m);
        Ok(())
    }
}

/// The source offset reported for a call: the property name of a method call, else the call.
fn call_position(c: &CallExpression<'_>) -> u32 {
    match c.callee.without_parentheses() {
        Expression::StaticMemberExpression(s) => s.property.span.start,
        _ => c.span.start,
    }
}
