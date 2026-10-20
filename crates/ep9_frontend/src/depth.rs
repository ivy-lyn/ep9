//! Depth guard layer 2: the AST-depth pass, and the depth limits (01-front-end §6, H23).
//!
//! ```text
//! layer 0  pre-scan (prescan.rs)   tokens only, before oxc     -> estimate > PRESCAN_CAP: RangeError
//! layer 1  oxc parser counter      not in oxc 0.155 (M1 patch)  -> the stack is sized for layer 0
//! layer 2  AST-depth pass (here)   after parse, before semantic -> depth > AST_CAP: RangeError
//! layer 3  our passes              explicit depth checks        -> depth > AST_CAP: RangeError
//! ```
//!
//! - Depth unit: AST nodes on the path from `Program` (depth 1) down. Every node that has an oxc
//!   `AstKind` counts once: `[[0]]` has depth 5 (Program, ExpressionStatement, 2 arrays, literal).
//! - The pass is a bounded recursion: it stops at every point where the AST type graph has a cycle
//!   (expressions, statements, binding patterns, assignment targets, TS types) once the depth
//!   passes the cap. So it never goes more than a few frames past the cap, even on a 10^6-deep
//!   left-leaning chain (`1+1+...`, `a.b.c...`, `f()()...`) that the parser builds in a loop.

use oxc_ast::AstKind;
use oxc_ast::ast::{AssignmentTarget, BindingPattern, Expression, Program, Statement, TSType};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;

use crate::ByteSpan;

/// Highest accepted AST depth (layer 2 and layer 3). Depth `AST_CAP` compiles;
/// depth `AST_CAP + 1` is a RangeError.
///
/// 20,000 is the plan default (01-front-end §6), above tsrs's 18.6k arrays on 8 MiB and above
/// V8's limits for every nesting form (node 24.22: 1,064 arrows to 12,392 unary operators).
pub const AST_CAP: u32 = 20_000;

/// Highest accepted pre-scan estimate (layer 0). See `prescan.rs` for the unit.
///
/// For every measured nesting form the estimate grows at most 1 unit per AST level, so a program
/// at depth [`AST_CAP`] has an estimate near `AST_CAP` (checked per form by the depth tests). The
/// extra 25% covers mixed forms. Inputs between the two caps parse and then fail in layer 2; the
/// worker stack fits a parse at this cap (`stack.rs`).
pub const PRESCAN_CAP: u32 = AST_CAP + AST_CAP / 4;

/// Which layer of the depth guard rejected the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DepthLayer {
    /// Layer 0: the token pre-scan, before oxc runs.
    PreScan,
    /// Layer 2: the AST-depth pass, after the parse and before oxc_semantic.
    Ast,
    /// Layer 3: an ep9 pass (scope pass, later BytecodeGen).
    Pass,
}

/// Result of the AST-depth pass when the AST is too deep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooDeep {
    /// The first node found below the cap.
    pub(crate) span: ByteSpan,
}

/// Layer 2: measure the AST depth of `program`.
///
/// Returns the maximum depth if it is at most `cap`.
///
/// # Errors
///
/// Returns [`TooDeep`] with the first node deeper than `cap`.
pub(crate) fn check_ast_depth(program: &Program<'_>, cap: u32) -> Result<u32, TooDeep> {
    let mut pass = DepthPass {
        depth: 0,
        max: 0,
        cap,
        too_deep: None,
    };
    pass.visit_program(program);
    match pass.too_deep {
        None => Ok(pass.max),
        Some(span) => Err(TooDeep { span }),
    }
}

struct DepthPass {
    depth: u32,
    max: u32,
    cap: u32,
    too_deep: Option<ByteSpan>,
}

impl DepthPass {
    /// False once the current path is below the cap: the caller then skips the subtree.
    #[inline]
    fn may_descend(&self) -> bool {
        self.depth <= self.cap
    }
}

impl<'a> Visit<'a> for DepthPass {
    #[inline]
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.depth += 1;
        if self.depth > self.max {
            self.max = self.depth;
            if self.depth > self.cap && self.too_deep.is_none() {
                let span = kind.span();
                self.too_deep = Some(ByteSpan {
                    start: span.start,
                    end: span.end,
                });
            }
        }
    }

    #[inline]
    fn leave_node(&mut self, _kind: AstKind<'a>) {
        self.depth -= 1;
    }

    // The cycle points of the AST type graph. Every recursive path passes through one of them.

    fn visit_expression(&mut self, it: &Expression<'a>) {
        if self.may_descend() {
            walk::walk_expression(self, it);
        }
    }

    fn visit_statement(&mut self, it: &Statement<'a>) {
        if self.may_descend() {
            walk::walk_statement(self, it);
        }
    }

    fn visit_binding_pattern(&mut self, it: &BindingPattern<'a>) {
        if self.may_descend() {
            walk::walk_binding_pattern(self, it);
        }
    }

    fn visit_assignment_target(&mut self, it: &AssignmentTarget<'a>) {
        if self.may_descend() {
            walk::walk_assignment_target(self, it);
        }
    }

    fn visit_ts_type(&mut self, it: &TSType<'a>) {
        if self.may_descend() {
            walk::walk_ts_type(self, it);
        }
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::*;

    fn depth(src: &str, cap: u32) -> Result<u32, TooDeep> {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, src, SourceType::script()).parse();
        assert!(!parsed.diagnostics.has_errors(), "{src}");
        check_ast_depth(&parsed.program, cap)
    }

    #[test]
    fn counts_every_ast_node_on_the_path() {
        // Program > ExpressionStatement > NumericLiteral
        assert_eq!(depth("0", 100), Ok(3));
        // Program > ExpressionStatement > ArrayExpression x2 > NumericLiteral
        assert_eq!(depth("[[0]]", 100), Ok(5));
        // Each paren adds a ParenthesizedExpression.
        assert_eq!(depth("((0))", 100), Ok(5));
        assert_eq!(depth("", 100), Ok(1));
    }

    #[test]
    fn the_cap_is_inclusive() {
        assert_eq!(depth("[[0]]", 5), Ok(5));
        let err = depth("[[0]]", 4).unwrap_err();
        // The literal is the first node at depth 5.
        assert_eq!(err.span, ByteSpan { start: 2, end: 3 });
    }

    #[test]
    fn left_deep_chains_stop_at_the_cap() {
        // 200k-term chains: the parser builds them in a loop; the pass must stop near the cap.
        // The default test thread stack (2 MiB) would overflow on a full recursive walk.
        for src in [
            format!("1{}", "+1".repeat(200_000)),
            format!("a{}", ".b".repeat(200_000)),
            format!("f{}", "()".repeat(200_000)),
        ] {
            let result = std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || depth(&src, 1_000))
                .unwrap()
                .join()
                .unwrap();
            assert!(result.is_err(), "{result:?}");
        }
    }
}
