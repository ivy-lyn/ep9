//! A static check of the context chain (test only).
//!
//! The ep9_bytecode verifier checks context operands only for range. This check follows the
//! context values through each function (data flow over registers, the accumulator and the
//! current context) and checks that the chain matches the scope descriptors:
//!
//! ```text
//! entry context      = nearest needs-context scope at or above Function.outer_scope (- = outside the unit)
//! Create*Context(s)  : current context == nearest needs-context scope above s
//! CreateClosure(f)   : current context == entry context of f
//! Lda/StaContextSlot : the register holds a known context; `depth` hops stay in the unit;
//!                      the slot is in the target scope
//! *CurrentContextSlot: the current context is a unit scope; the slot is in it
//! PushContext        : acc holds a known context
//! PopContext, handler: the register holds a known context
//! ```

use std::collections::BTreeMap;

use ep9_bytecode::{
    CompiledUnit, Constant, Function, Instruction, Instructions, Opcode, OpcodeFlags, OperandKind,
    Register, ScopeFlags, ScopeId, VerifiedUnit,
};

/// A value in the check: a context (its scope, `None` for the context outside the unit), or
/// anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cv {
    Ctx(Option<u32>),
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    current: Cv,
    acc: Cv,
    regs: Vec<Cv>,
}

impl State {
    /// Joins `other` into `self`; returns true on a change.
    fn join(&mut self, other: &State) -> bool {
        fn j(a: &mut Cv, b: Cv) -> bool {
            if *a != b && *a != Cv::Other {
                *a = Cv::Other;
                return true;
            }
            false
        }
        let mut changed = j(&mut self.current, other.current);
        changed |= j(&mut self.acc, other.acc);
        for (a, &b) in self.regs.iter_mut().zip(&other.regs) {
            changed |= j(a, b);
        }
        changed
    }

    fn reg(&self, r: Register) -> Cv {
        r.local_index()
            .and_then(|i| self.regs.get(i as usize).copied())
            .unwrap_or(Cv::Other)
    }

    fn set(&mut self, r: Register, v: Cv) {
        if let Some(i) = r.local_index()
            && let Some(slot) = self.regs.get_mut(i as usize)
        {
            *slot = v;
        }
    }
}

/// The nearest scope at or above `s` that has a context.
fn context_at(unit: &CompiledUnit, mut s: Option<ScopeId>) -> Option<u32> {
    while let Some(id) = s {
        let scope = unit.scope(id);
        if scope.flags.contains(ScopeFlags::NEEDS_CONTEXT) {
            return Some(id.0);
        }
        s = scope.outer;
    }
    None
}

/// The parent context of a context for scope `s`.
fn context_parent(unit: &CompiledUnit, s: u32) -> Option<u32> {
    context_at(unit, unit.scope(ScopeId(s)).outer)
}

fn show(c: Option<u32>) -> String {
    c.map_or_else(|| "-".to_string(), |s| format!("scope{s}"))
}

struct Checker<'u> {
    unit: &'u CompiledUnit,
    fi: usize,
    f: &'u Function,
    /// `None` while the fixed point runs; the error list in the final pass.
    errors: Option<Vec<String>>,
}

impl Checker<'_> {
    fn error(&mut self, insn: &Instruction<'_>, msg: String) {
        if let Some(errors) = &mut self.errors {
            errors.push(format!(
                "f{} @{} {:?}: {msg}",
                self.fi, insn.offset, insn.opcode
            ));
        }
    }

    fn scope_operand(&self, insn: &Instruction<'_>, i: usize) -> u32 {
        match self.f.constants[insn.operand(i) as usize] {
            Constant::Scope(s) => s.0,
            _ => unreachable!("verified: a Scope constant"),
        }
    }

    /// Checks a context value used by `insn` and returns its scope.
    fn known(&mut self, insn: &Instruction<'_>, what: &str, v: Cv) -> Option<Option<u32>> {
        match v {
            Cv::Ctx(c) => Some(c),
            Cv::Other => {
                self.error(insn, format!("{what} is not a known context"));
                None
            }
        }
    }

    /// Checks slot `slot` of the context `depth` hops out from `from`.
    fn slot(&mut self, insn: &Instruction<'_>, from: Option<u32>, depth: u32, slot: u32) {
        let mut c = from;
        for _ in 0..depth {
            match c {
                Some(s) => c = context_parent(self.unit, s),
                None => break,
            }
        }
        match c {
            None => self.error(
                insn,
                format!("{depth} hops from {} leave the unit", show(from)),
            ),
            Some(s) => {
                let n = self.unit.scope(ScopeId(s)).slots.len();
                if slot as usize >= n {
                    self.error(insn, format!("slot {slot} not in scope{s} ({n} slots)"));
                }
            }
        }
    }

    /// Applies `insn` to `st` (checks in the final pass).
    fn step(&mut self, insn: &Instruction<'_>, st: &mut State) {
        let op = insn.opcode;
        match op {
            Opcode::Star => st.set(insn.register(0), st.acc),
            Opcode::Ldar => st.acc = st.reg(insn.register(0)),
            Opcode::Mov => st.set(insn.register(1), st.reg(insn.register(0))),
            Opcode::LdaCurrentContext => st.acc = st.current,
            Opcode::PushContext => {
                let acc = st.acc;
                if self.known(insn, "acc", acc) == Some(None) {
                    self.error(insn, "pushes the context outside the unit".to_string());
                }
                st.set(insn.register(0), st.current);
                st.current = acc;
            }
            Opcode::PopContext => {
                let v = st.reg(insn.register(0));
                self.known(insn, "the register", v);
                st.current = v;
            }
            Opcode::CreateFunctionContext
            | Opcode::CreateBlockContext
            | Opcode::CreateEvalContext
            | Opcode::CreateCatchContext
            | Opcode::CreateWithContext => {
                let at = usize::from(matches!(
                    op,
                    Opcode::CreateCatchContext | Opcode::CreateWithContext
                ));
                let s = self.scope_operand(insn, at);
                if !self
                    .unit
                    .scope(ScopeId(s))
                    .flags
                    .contains(ScopeFlags::NEEDS_CONTEXT)
                {
                    self.error(insn, format!("scope{s} has no needs-context flag"));
                }
                let want = context_parent(self.unit, s);
                if let Some(cur) = self.known(insn, "the current context", st.current)
                    && cur != want
                {
                    self.error(
                        insn,
                        format!(
                            "scope{s} needs parent {} but the current context is {}",
                            show(want),
                            show(cur)
                        ),
                    );
                }
                st.acc = Cv::Ctx(Some(s));
            }
            Opcode::CreateClosure => {
                let Constant::Function(fid) = self.f.constants[insn.operand(0) as usize] else {
                    unreachable!("verified: a Function constant")
                };
                let want = context_at(self.unit, self.unit.function(fid).outer_scope);
                if let Some(cur) = self.known(insn, "the current context", st.current)
                    && cur != want
                {
                    self.error(
                        insn,
                        format!(
                            "{fid} needs context {} but the current context is {}",
                            show(want),
                            show(cur)
                        ),
                    );
                }
                st.acc = Cv::Other;
            }
            Opcode::LdaContextSlot | Opcode::LdaImmutableContextSlot | Opcode::StaContextSlot => {
                let v = st.reg(insn.register(0));
                if let Some(from) = self.known(insn, "the context register", v) {
                    self.slot(insn, from, insn.unsigned(2), insn.unsigned(1));
                }
                if op != Opcode::StaContextSlot {
                    st.acc = Cv::Other;
                }
            }
            Opcode::LdaCurrentContextSlot
            | Opcode::LdaImmutableCurrentContextSlot
            | Opcode::StaCurrentContextSlot => {
                let cur = st.current;
                if let Some(from) = self.known(insn, "the current context", cur) {
                    self.slot(insn, from, 0, insn.unsigned(0));
                }
                if op != Opcode::StaCurrentContextSlot {
                    st.acc = Cv::Other;
                }
            }
            _ => {
                let info = op.info();
                for (i, o) in info.operands.iter().enumerate() {
                    match o.kind {
                        OperandKind::RegOut | OperandKind::RegInOut => {
                            st.set(insn.register(i), Cv::Other);
                        }
                        OperandKind::RegOutList => {
                            let first = insn.register(i).to_operand();
                            for k in 0..insn.unsigned(i + 1) as i32 {
                                st.set(Register::from_operand(first + k), Cv::Other);
                            }
                        }
                        OperandKind::RegOutTriple => {
                            let first = insn.register(i).to_operand();
                            for k in 0..3 {
                                st.set(Register::from_operand(first + k), Cv::Other);
                            }
                        }
                        _ => {}
                    }
                }
                if matches!(
                    info.acc,
                    ep9_bytecode::AccUse::Write | ep9_bytecode::AccUse::ReadWrite
                ) || info.flags.contains(OpcodeFlags::CLOBBER)
                {
                    st.acc = Cv::Other;
                }
            }
        }
    }
}

/// Successor offsets of `insn` in normal control flow.
fn successors(f: &Function, insn: &Instruction<'_>) -> Vec<u32> {
    let flags = insn.opcode.info().flags;
    let mut out = Vec::new();
    if let Some(t) = insn.jump_target() {
        out.push(t);
    }
    if flags.contains(OpcodeFlags::SWITCH) {
        for (i, o) in insn.opcode.info().operands.iter().enumerate() {
            if o.kind == OperandKind::Const
                && let Constant::JumpTable(t) = &f.constants[insn.operand(i) as usize]
            {
                out.extend(t.iter().map(|&e| insn.offset + e));
            }
        }
    }
    if !flags.contains(OpcodeFlags::TERMINATOR) {
        out.push(insn.next_offset());
    }
    out
}

/// Context chain errors of `unit` (empty when the chain is consistent).
pub(crate) fn context_errors(unit: &VerifiedUnit) -> Vec<String> {
    let unit = unit.unit();
    let mut errors = Vec::new();
    for (fi, f) in unit.functions.iter().enumerate() {
        check_function(unit, fi, f, &mut errors);
    }
    errors
}

fn check_function(unit: &CompiledUnit, fi: usize, f: &Function, errors: &mut Vec<String>) {
    let insns: BTreeMap<u32, Instruction<'_>> = Instructions::new(&f.bytecode)
        .map(|i| {
            let i = i.expect("verified code decodes");
            (i.offset, i)
        })
        .collect();
    // The innermost handler of each throwing instruction (the last entry that holds it).
    let handler_of = |offset: u32| {
        f.handlers
            .iter()
            .rev()
            .find(|h| h.start <= offset && offset < h.end)
    };
    let entry = State {
        current: Cv::Ctx(context_at(unit, f.outer_scope)),
        acc: Cv::Other,
        regs: vec![Cv::Other; f.register_count as usize],
    };
    let mut cx = Checker {
        unit,
        fi,
        f,
        errors: None,
    };
    let mut states: BTreeMap<u32, State> = BTreeMap::from([(0, entry)]);
    let mut work = vec![0u32];
    while let Some(at) = work.pop() {
        let insn = insns[&at];
        let mut st = states[&at].clone();
        let mut next = Vec::new();
        if insn.opcode.info().flags.contains(OpcodeFlags::THROWS)
            && let Some(h) = handler_of(at)
        {
            let mut hs = st.clone();
            hs.current = st.reg(h.context);
            hs.acc = Cv::Other;
            next.push((h.target, hs));
        }
        cx.step(&insn, &mut st);
        for t in successors(f, &insn) {
            next.push((t, st.clone()));
        }
        for (t, s) in next {
            match states.get_mut(&t) {
                Some(old) => {
                    if old.join(&s) {
                        work.push(t);
                    }
                }
                None => {
                    states.insert(t, s);
                    work.push(t);
                }
            }
        }
    }
    // Final pass over the fixed point.
    cx.errors = Some(Vec::new());
    for (at, st) in &states {
        let insn = insns[at];
        if insn.opcode.info().flags.contains(OpcodeFlags::THROWS)
            && let Some(h) = handler_of(*at)
        {
            let v = st.reg(h.context);
            cx.known(&insn, "the handler context register", v);
        }
        cx.step(&insn, &mut st.clone());
    }
    errors.extend(cx.errors.take().unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use ep9_bytecode::{ScopeId, VerifiedUnit, verify};

    use super::context_errors;
    use crate::{SourceOpts, Strictness};

    const SRC: &str = "function f(a) { let b = 1; { let c = 2; return () => a + b + c; } }";

    fn unit() -> VerifiedUnit {
        let opts = SourceOpts::script(Strictness::Sloppy);
        super::super::compile(SRC, opts, super::super::CompileOptions::default()).unwrap()
    }

    /// Re-verifies a changed unit: the verifier accepts it, so only this check can see it.
    fn changed(edit: impl FnOnce(&mut ep9_bytecode::CompiledUnit)) -> Vec<String> {
        let mut u = unit().into_inner();
        edit(&mut u);
        context_errors(&verify(u).expect("the verifier accepts the change"))
    }

    #[test]
    fn the_compiled_unit_is_clean() {
        assert_eq!(context_errors(&unit()), Vec::<String>::new());
    }

    /// The arrow is created in the block context; the script scope is the wrong outer scope.
    #[test]
    fn catches_a_closure_in_the_wrong_context() {
        let errors = changed(|u| {
            u.functions.last_mut().unwrap().outer_scope = Some(ScopeId(0));
        });
        assert!(
            errors.iter().any(|e| e.contains("CreateClosure")),
            "{errors:?}"
        );
    }

    /// A block context whose parent is not the current context.
    #[test]
    fn catches_a_wrong_parent() {
        let errors = changed(|u| {
            let block = u
                .scopes
                .iter()
                .rposition(|s| {
                    s.kind == ep9_bytecode::ScopeKind::Block
                        && s.flags.contains(ep9_bytecode::ScopeFlags::NEEDS_CONTEXT)
                })
                .unwrap();
            u.scopes[block].outer = Some(ScopeId(0));
        });
        assert!(
            errors.iter().any(|e| e.contains("needs parent")),
            "{errors:?}"
        );
    }

    /// A slot read past the end of its scope.
    #[test]
    fn catches_a_slot_out_of_range() {
        let errors = changed(|u| {
            let block = u
                .scopes
                .iter()
                .rposition(|s| s.flags.contains(ep9_bytecode::ScopeFlags::NEEDS_CONTEXT))
                .unwrap();
            u.scopes[block].slots.clear();
        });
        assert!(
            errors.iter().any(|e| e.contains("not in scope")),
            "{errors:?}"
        );
    }
}
