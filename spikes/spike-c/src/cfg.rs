//! The graph builder's first steps: the reachable bytecode of one function as basic blocks,
//! and the liveness of every register at every instruction (H02-Q1 = "computed by the graph
//! builder", V9-D32). The liveness feeds the loop phis and the frame state of each guard.
//!
//! ```text
//!  bytecode ──decode from 0 along fall-through and jump edges──> reachable instructions
//!     leaders: 0, every jump target, every instruction after a jump or a return
//!  ──> blocks in offset order; a JumpLoop edge is a back edge, its target a loop header
//!  ──> liveness (backward, to a fixed point): live-before of every instruction, over the
//!      dense register index: 0 = <this>, 1..=P = a0.., P+1.. = r0.., last = the accumulator
//! ```
//!
//! Reads and writes come from the opcode table (`OpcodeInfo::acc` and the `Reg`, `RegOut`,
//! `RegInOut` operands), so the liveness has no per-opcode list. A `Clobber` opcode leaves
//! the accumulator undefined (a write). A read list with its count (`RegList` then
//! `RegCount`, SPIKEC-9) reads each of its registers; the other list forms are refused (no
//! supported opcode has one).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use ep9_bytecode::{Opcode, OpcodeFlags, OperandKind, OperandScale, Register, decode_at};
use spike_b::emit::{FunctionInput, Refusal};

/// One reachable instruction.
#[derive(Clone, Debug)]
pub struct Insn {
    pub offset: u32,
    pub opcode: Opcode,
    pub scale: OperandScale,
    /// The offset after it.
    pub next: u32,
    /// Dense register indexes it reads ([`Cfg::index`]).
    pub reads: Vec<usize>,
    /// Dense register indexes it writes.
    pub writes: Vec<usize>,
    /// Its jump target, for a jump.
    pub target: Option<u32>,
}

impl Insn {
    /// True if control can go on at `next`.
    pub fn falls_through(&self) -> bool {
        !self.opcode.info().flags.contains(OpcodeFlags::TERMINATOR)
    }
}

/// A basic block of reachable instructions.
#[derive(Clone, Debug)]
pub struct CfgBlock {
    /// Offset of its first instruction.
    pub start: u32,
    /// Its instructions: indexes into [`Cfg::insns`].
    pub insns: std::ops::Range<usize>,
    /// Successor block indexes (for a conditional jump: fall-through first, then the target).
    pub succs: Vec<usize>,
    pub preds: Vec<usize>,
    /// True if a `JumpLoop` targets it.
    pub loop_header: bool,
}

/// The control-flow graph and the liveness of one function.
#[derive(Clone, Debug)]
pub struct Cfg {
    pub insns: Vec<Insn>,
    pub blocks: Vec<CfgBlock>,
    pub parameter_count: u32,
    pub register_count: u32,
    /// `live_before[i][r]`: register `r` is live before instruction `i`.
    pub live_before: Vec<Vec<bool>>,
    /// The liveness time (the H02-Q1 report).
    pub liveness_time: Duration,
}

impl Cfg {
    /// Registers in the dense index (receiver, parameters, locals, accumulator).
    pub fn width(&self) -> usize {
        Self::width_of(self.parameter_count, self.register_count)
    }

    fn width_of(params: u32, regs: u32) -> usize {
        2 + params as usize + regs as usize
    }

    /// The dense index of the accumulator.
    pub fn acc(&self) -> usize {
        self.width() - 1
    }

    /// The dense index of `r`.
    pub fn index(&self, r: Register) -> usize {
        index_of(self.parameter_count, r)
    }

    /// The register of dense index `i` (not the accumulator).
    pub fn register(&self, i: usize) -> Register {
        let p = self.parameter_count as usize;
        if i == 0 {
            Register::RECEIVER
        } else if i <= p {
            Register::parameter((i - 1) as u32)
        } else {
            Register::local((i - 1 - p) as u32)
        }
    }

    /// The block that starts at `offset`.
    pub fn block_at(&self, offset: u32) -> Option<usize> {
        self.blocks.iter().position(|b| b.start == offset)
    }

    /// The H02-Q1 size of the liveness as one bit per register per instruction, in bytes.
    pub fn liveness_bytes(&self) -> usize {
        self.insns.len() * self.width().div_ceil(8)
    }

    /// True if `registers` and `acc` are exactly the live set before instruction `i`: every
    /// live register once, no other register, and the accumulator if and only if it is live
    /// (the H02-Q1 report compares each exit's frame state this way; a count compare would
    /// pass two sets of one size).
    pub fn is_live_set(&self, i: usize, registers: &[Register], acc: bool) -> bool {
        let mut got = vec![false; self.width()];
        for &r in registers {
            let k = self.index(r);
            if k >= self.acc() || got[k] {
                return false;
            }
            got[k] = true;
        }
        got[self.acc()] = acc;
        self.live_before.get(i) == Some(&got)
    }

    /// Decodes the reachable instructions of `input`, splits them into blocks and computes
    /// the liveness. `supported` says which opcodes the builder translates; any other
    /// reachable one is refused here.
    pub fn build(input: &FunctionInput<'_>, supported: fn(Opcode) -> bool) -> Result<Cfg, Refusal> {
        if input.handler_count != 0 {
            return Err(Refusal::Handlers(input.handler_count));
        }
        let code = input.bytecode;
        let len = code.len() as u32;
        let params = input.parameter_count;
        let regs = input.register_count;
        let width = Self::width_of(params, regs);
        let mut found: BTreeMap<u32, Insn> = BTreeMap::new();
        let mut work = vec![0u32];
        while let Some(off) = work.pop() {
            if found.contains_key(&off) {
                continue;
            }
            let insn = decode_at(code, off).map_err(|e| Refusal::Decode {
                offset: off,
                detail: format!("{e:?}"),
            })?;
            if !supported(insn.opcode) {
                return Err(Refusal::Opcode {
                    offset: off,
                    opcode: insn.opcode,
                });
            }
            let info = insn.opcode.info();
            let (mut reads, mut writes) = (Vec::new(), Vec::new());
            for (i, o) in info.operands.iter().enumerate() {
                let r = || {
                    let reg = insn.register(i);
                    let ix = index_of(params, reg);
                    if ix + 1 >= width {
                        Err(Refusal::Decode {
                            offset: off,
                            detail: format!("register {reg} out of range"),
                        })
                    } else {
                        Ok(ix)
                    }
                };
                match o.kind {
                    OperandKind::Reg => reads.push(r()?),
                    OperandKind::RegOut => writes.push(r()?),
                    OperandKind::RegInOut => {
                        let x = r()?;
                        reads.push(x);
                        writes.push(x);
                    }
                    // A read list (SPIKEC-9: `CallUndefinedReceiver r, rA-rB`): the count is
                    // the next operand (`RegCount`); every register of the list is read.
                    OperandKind::RegList
                        if info.operands.get(i + 1).map(|o| o.kind)
                            == Some(OperandKind::RegCount) =>
                    {
                        let first = insn.register(i);
                        let count = insn.unsigned(i + 1);
                        let base = first.local_index().ok_or_else(|| Refusal::Decode {
                            offset: off,
                            detail: format!("register list at {first}, not a local"),
                        })?;
                        for k in 0..count {
                            let ix = index_of(params, Register::local(base + k));
                            if ix + 1 >= width {
                                return Err(Refusal::Decode {
                                    offset: off,
                                    detail: format!("register list {first}+{count} out of range"),
                                });
                            }
                            reads.push(ix);
                        }
                    }
                    OperandKind::RegList
                    | OperandKind::RegOutList
                    | OperandKind::RegPair
                    | OperandKind::RegOutTriple => {
                        return Err(Refusal::Limit(format!(
                            "register list at {off} ({})",
                            info.name
                        )));
                    }
                    _ => {}
                }
            }
            let acc = width - 1;
            if info.acc.reads() {
                reads.push(acc);
            }
            if info.acc.writes() || info.flags.contains(OpcodeFlags::CLOBBER) {
                writes.push(acc);
            }
            let target = match insn.jump_target() {
                Some(t) if t < len => Some(t),
                Some(_) => {
                    return Err(Refusal::Decode {
                        offset: off,
                        detail: "jump target outside the bytecode".into(),
                    });
                }
                None => None,
            };
            let i = Insn {
                offset: off,
                opcode: insn.opcode,
                scale: insn.scale,
                next: insn.next_offset(),
                reads,
                writes,
                target,
            };
            if let Some(t) = target {
                work.push(t);
            }
            if i.falls_through() {
                if i.next >= len {
                    return Err(Refusal::Decode {
                        offset: off,
                        detail: "control falls off the end".into(),
                    });
                }
                work.push(i.next);
            }
            found.insert(off, i);
        }
        let insns: Vec<Insn> = found.into_values().collect();
        let at: BTreeMap<u32, usize> = insns
            .iter()
            .enumerate()
            .map(|(i, x)| (x.offset, i))
            .collect();
        // Leaders.
        let mut leaders = BTreeSet::from([0u32]);
        for x in &insns {
            if let Some(t) = x.target {
                leaders.insert(t);
                leaders.insert(x.next);
            } else if !x.falls_through() {
                leaders.insert(x.next);
            }
        }
        let mut blocks: Vec<CfgBlock> = Vec::new();
        for (i, x) in insns.iter().enumerate() {
            let new = i == 0 || leaders.contains(&x.offset) || insns[i - 1].next != x.offset;
            if new {
                blocks.push(CfgBlock {
                    start: x.offset,
                    insns: i..i + 1,
                    succs: Vec::new(),
                    preds: Vec::new(),
                    loop_header: false,
                });
            } else {
                blocks.last_mut().expect("a block").insns.end = i + 1;
            }
        }
        let block_of: BTreeMap<u32, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.start, i))
            .collect();
        for bi in 0..blocks.len() {
            let last = &insns[blocks[bi].insns.end - 1];
            let mut succs = Vec::new();
            if last.falls_through() {
                let n = *block_of.get(&last.next).ok_or_else(|| Refusal::Decode {
                    offset: last.offset,
                    detail: "falls into an unreachable offset".into(),
                })?;
                succs.push(n);
            }
            if let Some(t) = last.target {
                let n = *block_of.get(&t).ok_or_else(|| Refusal::Decode {
                    offset: last.offset,
                    detail: "jump into the middle of an instruction".into(),
                })?;
                if last.opcode == Opcode::JumpLoop {
                    if t > last.offset {
                        return Err(Refusal::Limit("a JumpLoop that goes forward".into()));
                    }
                    blocks[n].loop_header = true;
                } else if t <= last.offset {
                    return Err(Refusal::Limit(format!(
                        "a backward jump at {} that is not a JumpLoop",
                        last.offset
                    )));
                }
                succs.push(n);
            }
            blocks[bi].succs = succs.clone();
            for s in succs {
                blocks[s].preds.push(bi);
            }
        }
        let _ = at;
        let t = Instant::now();
        let live_before = liveness(&insns, &blocks, width);
        let mut cfg = Cfg {
            insns,
            blocks,
            parameter_count: params,
            register_count: regs,
            live_before,
            liveness_time: Duration::ZERO,
        };
        cfg.liveness_time = t.elapsed();
        // Only the receiver and the parameters may be live at entry (locals start undefined,
        // and BytecodeGen never reads one before it writes it).
        let entry = &cfg.live_before[0];
        if let Some(r) = (cfg.parameter_count as usize + 1..width).find(|&r| entry[r]) {
            return Err(Refusal::Limit(format!(
                "register {r} is read before it is written"
            )));
        }
        Ok(cfg)
    }
}

/// The dense index of `r` for a function with `params` parameters.
fn index_of(params: u32, r: Register) -> usize {
    if r.is_receiver() {
        0
    } else if let Some(k) = r.parameter_index() {
        if k < params {
            1 + k as usize
        } else {
            usize::MAX - 1
        }
    } else {
        1 + params as usize + r.local_index().expect("a local") as usize
    }
}

/// Backward liveness to a fixed point; `live_before[i]` of every instruction.
fn liveness(insns: &[Insn], blocks: &[CfgBlock], width: usize) -> Vec<Vec<bool>> {
    let mut live_in: Vec<Vec<bool>> = vec![vec![false; width]; blocks.len()];
    let mut before: Vec<Vec<bool>> = vec![vec![false; width]; insns.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for bi in (0..blocks.len()).rev() {
            let b = &blocks[bi];
            let mut live = vec![false; width];
            for &s in &b.succs {
                for r in 0..width {
                    live[r] |= live_in[s][r];
                }
            }
            for i in b.insns.clone().rev() {
                for &w in &insns[i].writes {
                    live[w] = false;
                }
                for &r in &insns[i].reads {
                    live[r] = true;
                }
                before[i].clone_from(&live);
            }
            if live != live_in[bi] {
                live_in[bi] = live;
                changed = true;
            }
        }
    }
    before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Session;
    use spike_b::harness::Config;

    fn all(_: Opcode) -> bool {
        true
    }

    /// A counted loop: blocks, the loop header, the back edge, and the liveness at the loop
    /// header (the counter, the bound and the sum; not the temporaries).
    #[test]
    fn loop_blocks_and_liveness() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(
            "function sum(n) { var a = 0; for (var i = 0; i < n; i++) { a = (a + i) | 0; } return a; }",
            "l.js",
        )
        .result
        .expect("run");
        let input = s.function_input("sum").expect("input");
        let cfg = Cfg::build(&input, all).expect("cfg");
        let headers: Vec<usize> = (0..cfg.blocks.len())
            .filter(|&b| cfg.blocks[b].loop_header)
            .collect();
        assert_eq!(headers.len(), 1, "{cfg:#?}");
        let h = &cfg.blocks[headers[0]];
        assert_eq!(h.preds.len(), 2, "entry and back edge");
        let live = &cfg.live_before[h.insns.start];
        let named: Vec<String> = (0..cfg.acc())
            .filter(|&r| live[r])
            .map(|r| cfg.register(r).to_string())
            .collect();
        // a0 = n, and two locals (a, i); never the accumulator at a loop header here.
        assert_eq!(named.len(), 3, "{named:?}");
        assert!(named.contains(&"a0".to_string()), "{named:?}");
        assert!(!live[cfg.acc()]);
        assert!(cfg.liveness_bytes() > 0);
    }

    /// `is_live_set` compares register sets, not counts (the M5-P4 W1 reviewer F5; Codex
    /// M5-P4 r1): at the loop header the live set itself passes; the same count with one
    /// register swapped for a dead one, a register twice, a missing register, and a live
    /// accumulator each fail.
    #[test]
    fn the_live_set_compare_reads_each_register() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(
            "function sum(n) { var a = 0; for (var i = 0; i < n; i++) { a = (a + i) | 0; } return a; }",
            "l.js",
        )
        .result
        .expect("run");
        let input = s.function_input("sum").expect("input");
        let cfg = Cfg::build(&input, all).expect("cfg");
        let i = (0..cfg.blocks.len())
            .find(|&b| cfg.blocks[b].loop_header)
            .map(|b| cfg.blocks[b].insns.start)
            .expect("a loop header");
        let (live, dead): (Vec<usize>, Vec<usize>) =
            (0..cfg.acc()).partition(|&r| cfg.live_before[i][r]);
        let regs = |ks: &[usize]| ks.iter().map(|&k| cfg.register(k)).collect::<Vec<_>>();
        assert_eq!(live.len(), 3, "{live:?}");
        assert!(!dead.is_empty(), "{dead:?}");
        assert!(cfg.is_live_set(i, &regs(&live), false));
        let mut swapped = live.clone();
        swapped[0] = dead[0];
        assert!(!cfg.is_live_set(i, &regs(&swapped), false), "{swapped:?}");
        let twice = [live[0], live[0], live[1]];
        assert!(!cfg.is_live_set(i, &regs(&twice), false));
        assert!(!cfg.is_live_set(i, &regs(&live[1..]), false));
        assert!(!cfg.is_live_set(i, &regs(&live), true));
    }

    #[test]
    fn unsupported_opcode_is_refused() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run("function f(o) { return o.x(); }", "u.js")
            .result
            .expect("run");
        let input = s.function_input("f").expect("input");
        let e = Cfg::build(&input, |op| op != Opcode::CallProperty0).expect_err("refused");
        assert!(matches!(e, Refusal::Opcode { .. }), "{e:?}");
    }
}
