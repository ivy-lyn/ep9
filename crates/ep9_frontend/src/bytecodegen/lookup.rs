//! Calls whose callee is a dynamic lookup (`with`, sloppy direct eval): `f()` where `f`
//! resolves to [`Resolution::Dynamic`](crate::scope::Resolution::Dynamic). The callee and its
//! receiver come from one `CallRuntimeForPair(LoadLookupSlotForCall, name)`, so that
//! `with (o) f()` gets `this === o` (§13.3.6.1 EvaluateCall step 1.b, WithBaseObject), and the
//! call takes the property call path of `call`:
//!
//! ```text
//! LdaConstant name; Star r_name
//! CallRuntimeForPair LoadLookupSlotForCall, r_name, (r_callee, list[0] = receiver)
//! <arguments into list[1..]>
//! CallProperty r_callee, list
//! ```
//!
//! The pair is written straight into `callee` and `list[0]` when they are adjacent (both
//! callers allocate `callee` just before the list); otherwise into two temporaries and moved.
//!
//! Caller: `expr.rs` (`callee_and_receiver`).

use ep9_bytecode::{Register, RegisterList, RuntimeId};
use oxc_ast::ast::{Expression, IdentifierReference};

use super::func::{FnGen, Gen};

impl<'a> FnGen<'a, '_> {
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
        let list = self.temps(1 + n);
        let m = self.mark();
        let name = self.temp();
        let k = self.str_const(id.name.as_str());
        self.b.lda_constant(k).star(name);
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
        self.b.call_runtime_for_pair(
            RuntimeId::LoadLookupSlotForCall,
            RegisterList::new(name.local_index().expect("a temporary"), 1),
            1,
            out,
        );
        if !adjacent {
            self.b.mov(out.get(0), callee).mov(out.get(1), list.get(0));
        }
        self.release(m);
        Ok(Some(list))
    }
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
