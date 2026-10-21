//! Calls whose callee is a dynamic lookup (`with`, sloppy direct eval): `f()` where `f`
//! resolves to [`Resolution::Dynamic`]. The callee and its receiver come from one
//! `CallRuntimeForPair(LoadLookupSlotForCall, name, strict)`, so that
//! `with (o) f()` gets `this === o` (§13.3.6.1 EvaluateCall step 1.b, WithBaseObject), and the
//! call takes the property call path of `call`:
//!
//! ```text
//! LdaConstant name; Star r_name; LdaTrue|LdaFalse; Star r_strict
//! CallRuntimeForPair LoadLookupSlotForCall, r_name-r_strict, (r_callee, list[0] = receiver)
//! <arguments into list[1..]>
//! CallProperty r_callee, list
//! ```
//!
//! The pair is written straight into `callee` and `list[0]` when they are adjacent (both
//! callers allocate `callee` just before the list); otherwise into two temporaries and moved.
//!
//! Caller: `expr.rs` (`callee_and_receiver`).
//!
//! Assignments to a dynamic name (`x = v`, `x op= v`, `x &&= v`, `x++`, `var x = v`) evaluate
//! the reference before the right-hand side (§13.15.2 step 1.a, §13.4.2.1, §14.3.2.1), and
//! PutValue goes to that reference even if the right-hand side deletes or adds a binding
//! (test262 S11.13.1_A5_T1, unscopables-inc-dec.js). So the target is resolved first, into a
//! list of 4 temporaries that the read and the store use (runtime ids 29-31, ABI 4):
//!
//! ```text
//! LdaConstant name; Star r_name
//! CallRuntime ResolveLookupSlot, r_name; Star r_env       (Prepared::Dynamic)
//! [LdaTrue|LdaFalse; Star r_value                         (compound, logical, update:
//!  CallRuntime LoadLookupSlotResolved, r_env-r_value]       r_value holds the strictness)
//! <right-hand side>; Star r_value; LdaTrue|LdaFalse; Star r_strict
//! CallRuntime StoreLookupSlotResolved, r_env-r_strict     (acc = the value)
//! ```
//!
//! A destructuring target (assignment pattern, or a `var` binding pattern) is resolved the
//! same way before its value is read: before GetV, the iterator step and the default
//! (§13.15.5.3-6, §8.6.3, §8.6.4; `pattern.rs`).
//!
//! Callers: `expr.rs` (`assignment`, `update`, `get_prepared`, `put_prepared`), `stmt.rs`
//! (`var_decl`) and `pattern.rs` (`prepare_element`, the shorthand property).
//! `StaLookupSlot` (which resolves at the store) stays where no code runs between the
//! reference and the store, so both orders give the same result: a plain `=` or `var x =`
//! whose value is a literal or a function ([`runs_no_code`]), and a plain identifier head of
//! a for-in/of (`for (x of o)`, `for (var x in o)`: its store follows the value).
//!
//! GetValue of a dynamic name takes the strictness of the use site (§9.4.2 step 3): a `with`
//! binding that is gone by then reads as undefined in sloppy code and throws a ReferenceError
//! in strict code (§9.1.1.2.6 step 2). `LdaLookupSlot` and `LdaLookupSlotInsideTypeof` take the
//! strictness of the function in the VM, so a strict read in a sloppy function (a class
//! heritage or computed key, §11.2.2) uses the resolved form ([`FnGen::load_dynamic_strict`];
//! `typeof` of an unresolvable name gives undefined, §13.5.3.1 step 3.a):
//!
//! ```text
//! LdaConstant name; Star r_name; CallRuntime ResolveLookupSlot, r_name; Star r_env
//! [JumpIfNull L_undefined]                                 (typeof)
//! LdaTrue; Star r_value; CallRuntime LoadLookupSlotResolved, r_env-r_value
//! [Jump L_end; L_undefined: LdaUndefined; L_end:]          (typeof)
//! ```

use ep9_bytecode::{Register, RegisterList, RuntimeId};
use oxc_ast::ast::{Expression, IdentifierReference};

use super::expr::Prepared;
use super::func::{FnGen, Gen};
use crate::scope::{Reference, Resolution};

impl<'a, 's> FnGen<'a, 's> {
    /// Evaluates the callee `callee_expr` (the identifier `id`, maybe in parentheses) into
    /// `callee`. Returns `Some(list)` of `1 + n` registers with the receiver in `list[0]` for a
    /// property-style call, or `None` for a call with an undefined receiver.
    pub(crate) fn dynamic_callee(
        &mut self,
        callee_expr: &'a Expression<'a>,
        id: &'a IdentifierReference<'a>,
        callee: Register,
        n: u32,
    ) -> Gen<Option<RegisterList>> {
        // `(f)()`: a parenthesized reference is still a reference (§13.2.9.1), so the
        // receiver is the same; `callee_expr` only adds the parentheses.
        let _ = callee_expr;
        let strict = self.strict_at(self.reference_of(id)?);
        let list = self.temps(1 + n);
        let m = self.mark();
        let args = self.temps(2);
        let k = self.str_const(id.name.as_str());
        self.b.lda_constant(k).star(args.get(0));
        self.lda_bool(strict);
        self.b.star(args.get(1));
        let adjacent = matches!(
            (callee.local_index(), list.first().local_index()),
            (Some(c), Some(l)) if l == c + 1
        );
        let out = if adjacent {
            RegisterList::new(callee.local_index().expect("checked"), 2)
        } else {
            self.temps(2)
        };
        self.expr_pos(id.span.start);
        self.b
            .call_runtime_for_pair(RuntimeId::LoadLookupSlotForCall, args, 2, out);
        if !adjacent {
            self.b.mov(out.get(0), callee).mov(out.get(1), list.get(0));
        }
        self.release(m);
        Ok(Some(list))
    }

    /// Resolves `p` now when it is a dynamic name (see the module doc), so the read and the
    /// store that follow use this reference. Any other target is returned as it is.
    pub(crate) fn resolve_dynamic(&mut self, p: Prepared<'a, 's>) -> Prepared<'a, 's> {
        let Prepared::Binding(r) = p else {
            return p;
        };
        if !matches!(r.resolution, Resolution::Dynamic { .. }) {
            return p;
        }
        let regs = self.resolve_into_temps(r);
        Prepared::Dynamic { r, regs }
    }

    /// `regs[0] = ResolveLookupSlot(regs[1] = name of r)` in 4 new temporaries
    /// [environment, name, value, strict]; the accumulator holds the environment.
    fn resolve_into_temps(&mut self, r: &Reference) -> RegisterList {
        let regs = self.temps(4);
        let k = self.name_const(r.name);
        self.b.lda_constant(k).star(regs.get(1));
        self.expr_pos(r.span.start);
        self.runtime(RuntimeId::ResolveLookupSlot, sub_list(regs, 1, 1));
        self.b.star(regs.get(0));
        regs
    }

    /// `acc = GetValue` of a reference that [`Self::resolve_dynamic`] resolved into `regs`
    /// (`strict`: the read is in strict code). The strictness goes in the value register, which
    /// is free until the store.
    pub(crate) fn load_resolved(&mut self, at: u32, regs: RegisterList, strict: bool) {
        self.lda_bool(strict);
        self.b.star(regs.get(2));
        self.expr_pos(at);
        self.runtime(RuntimeId::LoadLookupSlotResolved, sub_list(regs, 0, 3));
    }

    /// `acc = GetValue` of the dynamic reference `r` in strict code inside a sloppy function
    /// (see the module doc): ResolveBinding, then (for `typeof`, unless unresolvable) a strict
    /// GetValue.
    pub(crate) fn load_dynamic_strict(&mut self, r: &Reference, inside_typeof: bool) {
        let m = self.mark();
        let regs = self.resolve_into_temps(r);
        let labels = inside_typeof.then(|| {
            let (undefined, end) = (self.b.new_label(), self.b.new_label());
            self.b.jump_if_null(undefined);
            (undefined, end)
        });
        self.load_resolved(r.span.start, regs, true);
        if let Some((undefined, end)) = labels {
            self.b.jump(end);
            self.b.bind(undefined);
            self.b.lda_undefined();
            self.b.bind(end);
        }
        self.release(m);
    }

    /// PutValue of the accumulator to that reference (`strict`: the store is in strict code).
    /// The accumulator holds the value afterwards.
    pub(crate) fn store_resolved(&mut self, at: u32, regs: RegisterList, strict: bool) {
        self.b.star(regs.get(2));
        self.lda_bool(strict);
        self.b.star(regs.get(3));
        self.expr_pos(at);
        self.runtime(RuntimeId::StoreLookupSlotResolved, regs);
    }

    /// `acc = true` or `false`.
    fn lda_bool(&mut self, v: bool) {
        if v {
            self.b.lda_true();
        } else {
            self.b.lda_false();
        }
    }
}

/// Evaluating `e` runs no JS code and changes no binding (a literal, a function or arrow
/// expression, a template with no substitutions): a dynamic name assigned from it can be
/// resolved at the store.
pub(crate) fn runs_no_code(e: &Expression<'_>) -> bool {
    match e.without_parentheses() {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_) => true,
        Expression::TemplateLiteral(t) => t.expressions.is_empty(),
        _ => false,
    }
}

/// `len` registers of `regs` from index `from`.
fn sub_list(regs: RegisterList, from: u32, len: u32) -> RegisterList {
    let first = regs
        .get(from)
        .local_index()
        .expect("temporaries are locals");
    RegisterList::new(first, len)
}

#[cfg(test)]
mod tests {
    use crate::{CompileOptions, SourceOpts, Strictness, compile};

    /// The instruction lines (text after the `S>`/`E>` marks) of the script function.
    fn code(src: &str) -> Vec<String> {
        let unit = compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .unwrap();
        let text = ep9_bytecode::disassemble(unit.unit());
        text.lines()
            .skip_while(|l| !l.starts_with("function f0 "))
            .skip(2)
            .take_while(|l| !l.starts_with("  constants"))
            .map(|l| {
                let l = l.split(" ; ").next().unwrap_or(l);
                let l = l.trim_start();
                let l = l.split_once(' ').map_or(l, |(_, rest)| rest).trim_start();
                l.trim_start_matches("S> ")
                    .trim_start_matches("E> ")
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// The opcode after `CallRuntimeForPair LoadLookupSlotForCall, r_name, rC-rR` (skipping
    /// argument setup) calls `rC` with the receiver `rR`.
    fn assert_pair_feeds_call(lines: &[String], call: &str) {
        let i = lines
            .iter()
            .position(|l| l.starts_with("CallRuntimeForPair LoadLookupSlotForCall,"))
            .unwrap_or_else(|| panic!("no pair call in {lines:#?}"));
        let out = lines[i].rsplit(", ").next().unwrap();
        let (c, r) = out.split_once('-').unwrap();
        let j = lines[i..]
            .iter()
            .position(|l| l.starts_with(call))
            .unwrap_or_else(|| panic!("no {call} after the pair in {lines:#?}"));
        let want = format!("{call} {c}, {r}");
        assert!(
            lines[i + j].starts_with(&want),
            "{} does not start with {want}",
            lines[i + j]
        );
    }

    /// `with (o) f(args)`: one LoadLookupSlotForCall writes the callee and the receiver
    /// straight into the call's registers, and the call takes the property path.
    #[test]
    fn with_call_gets_the_receiver() {
        let src = "var o = {f() {}}; with (o) { f(1, 2); }";
        let lines = code(src);
        assert_pair_feeds_call(&lines, "CallProperty2");
        assert!(
            !lines.iter().any(|l| l.starts_with("LdaLookupSlot")),
            "{lines:#?}"
        );
    }

    /// Parentheses keep the reference: `(f)()` in `with` is a property call too.
    #[test]
    fn parenthesized_callee_keeps_the_receiver() {
        assert_pair_feeds_call(&code("with ({}) { (f)(); }"), "CallProperty0");
    }

    /// Tagged templates and spread calls take the same path.
    #[test]
    fn tagged_template_and_spread() {
        assert_pair_feeds_call(&code("with ({}) { f`t`; }"), "CallProperty1");
        assert_pair_feeds_call(&code("with ({}) { f(...a); }"), "CallWithSpread");
    }

    /// The position of the first line that starts with `op`.
    fn at(lines: &[String], op: &str) -> usize {
        lines
            .iter()
            .position(|l| l.starts_with(op))
            .unwrap_or_else(|| panic!("no {op} in {lines:#?}"))
    }

    /// `x = v`, `x op= v`, `x ||= v`, `x++` and `var x = v` on a dynamic `x` resolve it
    /// before `v` runs (here the dynamic call `f()`), then read and store through it.
    #[test]
    fn a_dynamic_target_is_resolved_before_the_value() {
        let resolve = "CallRuntime ResolveLookupSlot";
        let load = "CallRuntime LoadLookupSlotResolved";
        let store = "CallRuntime StoreLookupSlotResolved";
        let call = "CallRuntimeForPair LoadLookupSlotForCall";
        for (src, reads) in [
            ("with (o) { x = f(); }", false),
            ("with (o) { x += f(); }", true),
            ("with (o) { x ||= f(); }", true),
            ("with (o) { var x = f(); }", false),
        ] {
            let lines = code(src);
            assert!(at(&lines, resolve) < at(&lines, call), "{src}: {lines:#?}");
            assert!(at(&lines, call) < at(&lines, store), "{src}: {lines:#?}");
            if reads {
                assert!(at(&lines, resolve) < at(&lines, load), "{src}: {lines:#?}");
                assert!(at(&lines, load) < at(&lines, call), "{src}: {lines:#?}");
            }
            assert!(
                !lines
                    .iter()
                    .any(|l| l.starts_with("StaLookupSlot") || l.starts_with("LdaLookupSlot k")),
                "{src}: {lines:#?}"
            );
        }
        let lines = code("with (o) { x++; }");
        assert!(at(&lines, resolve) < at(&lines, load), "{lines:#?}");
        assert!(at(&lines, load) < at(&lines, store), "{lines:#?}");
    }

    /// A value that runs no code keeps the one-step `StaLookupSlot` (both orders agree).
    #[test]
    fn a_literal_value_keeps_sta_lookup_slot() {
        let lines = code("with (o) { x = 1; var y = function () {}; z = `t`; }");
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with("StaLookupSlot"))
                .count(),
            3,
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("LookupSlotResolved")),
            "{lines:#?}"
        );
    }

    /// A named function expression's own name, when it lives in a context, is described
    /// as `function-name` (VariableMode::FunctionName, ABI 4): the VM's dynamic store must
    /// see that it is immutable but not strict.
    #[test]
    fn a_function_name_slot_has_its_own_mode() {
        let unit = compile(
            "var f = function g() { with ({}) { g = 1; } };",
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .unwrap();
        let text = ep9_bytecode::disassemble(unit.unit());
        assert!(text.contains("slots=[\"g\":function-name]"), "{text}");
    }

    /// GetValue takes the strictness of the use site (M2-P2 review fix L6): a read in a class
    /// heritage or computed key (strict code, §11.2.2) in a sloppy script resolves first and
    /// reads with `true` (with a null check for `typeof`), a sloppy read keeps
    /// `LdaLookupSlot`, and a dynamic call passes its strictness to `LoadLookupSlotForCall`.
    #[test]
    fn a_strict_read_in_sloppy_code_is_resolved() {
        let count = |lines: &[String], op: &str| lines.iter().filter(|l| l.starts_with(op)).count();
        let lines =
            code("with (o) { x; class C extends (y, Object) { [typeof z]() {} [f()]() {} } }");
        assert_eq!(count(&lines, "LdaLookupSlot "), 1, "{lines:#?}");
        assert_eq!(count(&lines, "LdaLookupSlotInsideTypeof"), 0, "{lines:#?}");
        // y, Object (also dynamic inside `with`) and z.
        assert_eq!(
            count(&lines, "CallRuntime ResolveLookupSlot"),
            3,
            "{lines:#?}"
        );
        assert_eq!(
            count(&lines, "CallRuntime LoadLookupSlotResolved"),
            3,
            "{lines:#?}"
        );
        assert_eq!(count(&lines, "JumpIfNull"), 1, "{lines:#?}");
        for (i, l) in lines.iter().enumerate() {
            if l.starts_with("CallRuntime LoadLookupSlotResolved") {
                assert_eq!(lines[i - 2], "LdaTrue", "{lines:#?}");
            }
        }
        let call = at(&lines, "CallRuntimeForPair LoadLookupSlotForCall");
        assert_eq!(lines[call - 2], "LdaTrue", "{lines:#?}");
        let sloppy = code("with (o) { f(); }");
        let call = at(&sloppy, "CallRuntimeForPair LoadLookupSlotForCall");
        assert_eq!(sloppy[call - 2], "LdaFalse", "{sloppy:#?}");
    }

    /// Destructuring targets resolve before their value (M2-P2 review fix L7): the shorthand
    /// `{ x }` and the `var` property `{ y }` resolve before their `GetNamedProperty`, the
    /// array element `z` before the iterator's `next` call, and all three store through the
    /// resolved reference.
    #[test]
    fn a_destructuring_target_is_resolved_before_its_value() {
        let lines = code("with (o) { ({ x } = v); var { y } = v; [z] = v; }");
        let resolves: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i].starts_with("CallRuntime ResolveLookupSlot"))
            .collect();
        assert_eq!(resolves.len(), 3, "{lines:#?}");
        let after = |i: usize, op: &str| {
            i + lines[i..]
                .iter()
                .position(|l| l.starts_with(op))
                .unwrap_or_else(|| panic!("no {op} after {i} in {lines:#?}"))
        };
        assert!(
            after(resolves[0], "GetNamedProperty") < resolves[1],
            "{lines:#?}"
        );
        assert!(
            after(resolves[1], "GetNamedProperty") < resolves[2],
            "{lines:#?}"
        );
        let next = after(resolves[2], "CallProperty0");
        assert!(
            after(resolves[2], "CallRuntime StoreLookupSlotResolved") > next,
            "{lines:#?}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with("CallRuntime StoreLookupSlotResolved"))
                .count(),
            3,
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|l| l.starts_with("StaLookupSlot")),
            "{lines:#?}"
        );
    }

    /// A callee that is not a dynamic lookup keeps the plain call.
    #[test]
    fn static_callee_is_unchanged() {
        let lines = code("function f() {} f();");
        assert!(
            !lines.iter().any(|l| l.contains("LoadLookupSlotForCall")),
            "{lines:#?}"
        );
    }
}
