//! Register allocation (SPIKEC-1, M4-P1): linear scan (Poletto and Sarkar) over the SSA
//! values of an [`ir::Graph`], AArch64 registers only.
//!
//! | Register | Use (placement (b), spikes/spike-b/src/jit_b/a64.rs) | Allocatable |
//! |---|---|---|
//! | x0-x3 | runtime-table call arguments and result; the (b) exit's `(cx, pc, fp, acc)` | no |
//! | x4-x11 | values (caller-saved: the poll's slow path saves them around the call) | yes |
//! | x12-x14 | operand staging: an input that is not in a register is loaded here | no |
//! | x15-x17 | emitter temporaries (x16, x17 = IP0, IP1); x15 also stages a spilled result | no |
//! | x18 | the platform register (macOS) | no |
//! | x19-x25 | values (callee-saved; the prologue saves them) | yes |
//! | x26 | VM frame pointer | no |
//! | x27 | unit-data register (H35-Q9), saved with x25 | no |
//! | x28 | isolate (`Cx`) | no |
//! | x29, x30 | frame record | no |
//!
//! ```text
//!  linear order: blocks in bytecode order, live nodes in block order
//!  positions: block start S (its phis), nodes at S+2, S+4, ...; block end E (phi moves)
//!  SSA liveness per block (phi inputs count at the predecessor's end) ──> one interval per
//!  value [def, last use or end of the last block it is live out of] (no holes: a value live
//!  around a loop covers the whole loop)
//!  scan by start; no free register ──> spill the interval that ends last (whole lifetime on
//!  the native stack, one slot each)
//! ```
//!
//! Values that need no register: constants (the emitter makes them at each use), the
//! receiver and the parameters (read from their VM frame slot at each use: compiled code never
//! writes a VM frame slot, so the slot keeps the incoming Value), and a compare that the
//! branch right after it consumes (the emitter fuses them; its inputs are then used at the
//! branch). A value that lives across a guard has a location the deopt exit can read
//! (H29-Q2): a register, a native stack slot, a VM frame slot or a constant.

use crate::ir::{self, Graph, NodeId, Op, Repr};

/// Allocatable registers, in the order the scan takes them.
pub const REGS: [u8; 15] = [4, 5, 6, 7, 8, 9, 10, 11, 19, 20, 21, 22, 23, 24, 25];

/// Where a value lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    /// No value (effects, control) or a removed node.
    None,
    /// A machine register (`x<n>`; an Int32 or Bool in its low 32 bits).
    Reg(u8),
    /// Native stack slot `k` of the compiled function's frame.
    Stack(u32),
    /// A constant: made at each use.
    Const,
    /// A VM frame slot at this byte offset from fp (the receiver and the parameters).
    Frame(i32),
    /// A compare fused into the branch after it.
    Fused,
}

/// The allocation of one graph.
#[derive(Clone, Debug)]
pub struct Allocation {
    /// Per node id.
    pub loc: Vec<Location>,
    /// Native stack slots used.
    pub spill_slots: u32,
    /// Intervals made and spilled (the report).
    pub intervals: usize,
    pub spilled: usize,
    /// The largest number of values live in registers at one point.
    pub max_live: usize,
    /// The intervals: (first position, last position, value).
    pub ranges: Vec<(u32, u32, NodeId)>,
}

impl Allocation {
    pub fn of(&self, n: NodeId) -> Location {
        self.loc[n.0 as usize]
    }
}

/// True if the compare `n` is consumed by the branch right after it (only use, same block).
pub fn fused(g: &Graph, n: NodeId) -> bool {
    let node = g.node(n);
    if !matches!(node.op, Op::Compare(_)) {
        return false;
    }
    let b = g.block(node.block);
    let live: Vec<NodeId> = b
        .nodes
        .iter()
        .copied()
        .filter(|&x| g.node(x).live)
        .collect();
    let Some(k) = live.iter().position(|&x| x == n) else {
        return false;
    };
    let Some(&next) = live.get(k + 1) else {
        return false;
    };
    if g.node(next).op != Op::Branch || g.node(next).inputs != [n] {
        return false;
    }
    // No other use anywhere (frame states included).
    g.order().all(|m| {
        m == next
            || (!g.node(m).inputs.contains(&n)
                && g.node(m).guard.as_ref().is_none_or(|gd| {
                    gd.state.acc != Some(n) && gd.state.registers.iter().all(|&(_, v)| v != n)
                }))
    })
}

/// Allocates registers for `graph`.
pub fn allocate(g: &Graph) -> Result<Allocation, String> {
    let n = g.nodes.len();
    let mut loc = vec![Location::None; n];
    // Positions.
    let mut pos = vec![0u32; n];
    let mut block_start = vec![0u32; g.blocks.len()];
    let mut block_end = vec![0u32; g.blocks.len()];
    let mut p = 0u32;
    for (bi, b) in g.blocks.iter().enumerate() {
        block_start[bi] = p;
        for &x in &b.nodes {
            if !g.node(x).live {
                continue;
            }
            if g.node(x).op == Op::Phi {
                pos[x.0 as usize] = p;
            } else {
                p += 2;
                pos[x.0 as usize] = p;
            }
        }
        p += 2;
        block_end[bi] = p;
        p += 2;
    }
    let fused_set: Vec<bool> = (0..n as u32)
        .map(|i| g.node(NodeId(i)).live && fused(g, NodeId(i)))
        .collect();
    // Which values need a location.
    let needs = |x: NodeId| -> bool {
        let node = g.node(x);
        node.live
            && node.repr != Repr::None
            && !matches!(node.op, Op::Const(_) | Op::ConstInt32(_) | Op::Incoming(_))
            && !fused_set[x.0 as usize]
    };
    for i in 0..n {
        let x = NodeId(i as u32);
        let node = g.node(x);
        if !node.live {
            continue;
        }
        loc[i] = match node.op {
            Op::Const(_) | Op::ConstInt32(_) => Location::Const,
            Op::Incoming(ir::Incoming::Receiver) => {
                Location::Frame(ep9_vm::layout::FP_RECEIVER as i32)
            }
            Op::Incoming(ir::Incoming::Param(k)) => {
                Location::Frame(ep9_vm::layout::fp_param(k) as i32)
            }
            _ if fused_set[i] => Location::Fused,
            _ => Location::None,
        };
    }
    // Uses: (value, position). A fused compare's inputs are used at its branch.
    let mut start = vec![u32::MAX; n];
    let mut end = vec![0u32; n];
    let touch = |v: NodeId, at: u32, start: &mut Vec<u32>, end: &mut Vec<u32>| {
        let i = v.0 as usize;
        start[i] = start[i].min(at);
        end[i] = end[i].max(at);
    };
    for x in g.order() {
        let node = g.node(x);
        if needs(x) {
            touch(x, pos[x.0 as usize], &mut start, &mut end);
        }
        if node.op == Op::Phi {
            for (k, &v) in node.inputs.iter().enumerate() {
                let pred = g.block(node.block).preds[k];
                touch(v, block_end[pred.0 as usize], &mut start, &mut end);
            }
            continue;
        }
        let at = if fused_set[x.0 as usize] {
            // The branch right after it.
            pos[x.0 as usize] + 2
        } else {
            pos[x.0 as usize]
        };
        for &v in &node.inputs {
            touch(v, at, &mut start, &mut end);
        }
    }
    // SSA liveness per block: extend each value to the end of every block it is live out of.
    let nb = g.blocks.len();
    let mut live_in: Vec<Vec<bool>> = vec![vec![false; n]; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for bi in (0..nb).rev() {
            let b = &g.blocks[bi];
            let mut live = vec![false; n];
            for &s in &b.succs {
                let sb = g.block(s);
                let k = sb
                    .preds
                    .iter()
                    .position(|&q| q.0 as usize == bi)
                    .expect("a pred");
                for v in 0..n {
                    if live_in[s.0 as usize][v] && g.node(NodeId(v as u32)).op != Op::Phi {
                        live[v] = true;
                    }
                }
                for &ph in &sb.nodes {
                    if g.node(ph).live && g.node(ph).op == Op::Phi {
                        live[g.node(ph).inputs[k].0 as usize] = true;
                    }
                }
            }
            for &x in b.nodes.iter().rev() {
                let node = g.node(x);
                if !node.live {
                    continue;
                }
                live[x.0 as usize] = false;
                if node.op != Op::Phi {
                    for &v in &node.inputs {
                        live[v.0 as usize] = true;
                    }
                }
            }
            for &x in &b.nodes {
                if g.node(x).live && g.node(x).op == Op::Phi {
                    live[x.0 as usize] = true;
                }
            }
            if live != live_in[bi] {
                live_in[bi] = live;
                changed = true;
            }
        }
    }
    for (b, &bend) in g.blocks.iter().zip(&block_end) {
        for &s in &b.succs {
            for (v, &live) in live_in[s.0 as usize].iter().enumerate() {
                let x = NodeId(v as u32);
                if live && g.node(x).op != Op::Phi {
                    touch(x, bend, &mut start, &mut end);
                }
            }
        }
    }
    // Intervals of the values that need a location.
    let mut iv: Vec<(u32, u32, NodeId)> = (0..n as u32)
        .map(NodeId)
        .filter(|&x| needs(x) && start[x.0 as usize] != u32::MAX)
        .map(|x| (start[x.0 as usize], end[x.0 as usize], x))
        .collect();
    iv.sort();
    let intervals = iv.len();
    let mut free: Vec<u8> = REGS.iter().rev().copied().collect();
    let mut active: Vec<(u32, NodeId, u8)> = Vec::new(); // (end, node, reg)
    let mut spill_slots = 0u32;
    let mut spilled = 0usize;
    let mut max_live = 0usize;
    for &(s, e, x) in &iv {
        active.retain(|&(ae, _, r)| {
            if ae < s {
                free.push(r);
                false
            } else {
                true
            }
        });
        if let Some(r) = free.pop() {
            loc[x.0 as usize] = Location::Reg(r);
            active.push((e, x, r));
        } else {
            // Spill the one that ends last.
            let (k, &(ae, ax, ar)) = active
                .iter()
                .enumerate()
                .max_by_key(|(_, a)| a.0)
                .expect("active is full");
            if ae > e {
                loc[ax.0 as usize] = Location::Stack(spill_slots);
                loc[x.0 as usize] = Location::Reg(ar);
                active[k] = (e, x, ar);
            } else {
                loc[x.0 as usize] = Location::Stack(spill_slots);
            }
            spill_slots += 1;
            spilled += 1;
        }
        max_live = max_live.max(active.len());
    }
    Ok(Allocation {
        loc,
        spill_slots,
        intervals,
        spilled,
        max_live,
        ranges: iv,
    })
}

/// Checks the allocator's invariant: every register value is in an allocatable register, and
/// no two values whose intervals meet share a register (tests and `spike-c structural` run
/// it).
pub fn verify(g: &Graph, a: &Allocation) -> Result<(), String> {
    let mut by_reg: std::collections::BTreeMap<u8, Vec<(u32, u32, NodeId)>> = Default::default();
    for &(s, e, x) in &a.ranges {
        match a.of(x) {
            Location::Reg(r) if REGS.contains(&r) => by_reg.entry(r).or_default().push((s, e, x)),
            Location::Reg(r) => {
                return Err(format!("n{} in x{r}, not an allocatable register", x.0));
            }
            Location::Stack(k) if k < a.spill_slots => {}
            l => return Err(format!("n{} has an interval but is at {l:?}", x.0)),
        }
    }
    for x in g.order() {
        if matches!(a.of(x), Location::Reg(_) | Location::Stack(_))
            && !a.ranges.iter().any(|r| r.2 == x)
        {
            return Err(format!("n{} has a location but no interval", x.0));
        }
    }
    for (r, list) in by_reg {
        for (i, &(s1, e1, x1)) in list.iter().enumerate() {
            for &(s2, e2, x2) in &list[i + 1..] {
                if s1 <= e2 && s2 <= e1 {
                    return Err(format!(
                        "n{} [{s1}, {e1}] and n{} [{s2}, {e2}] share x{r}",
                        x1.0, x2.0
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With more values live at once than registers, the scan spills and every register
    /// holds at most one live value at a time.
    #[test]
    fn spills_when_full() {
        use crate::ir::{Block, BlockId, Trust};
        use ep9_facts::FactSource;
        let mut g = Graph::new("t", FactSource::Runtime, 1, 0);
        g.blocks.push(Block::default());
        let b = BlockId(0);
        let mut vals = Vec::new();
        let p = g.add(
            Op::Incoming(ir::Incoming::Param(0)),
            vec![],
            Repr::Tagged,
            Trust::Unknown,
            b,
            0,
        );
        g.blocks[0].nodes.push(p);
        for _ in 0..20 {
            let v = g.add(Op::CheckInt32, vec![p], Repr::Int32, Trust::Unknown, b, 0);
            g.blocks[0].nodes.push(v);
            vals.push(v);
        }
        // One use of all of them at the end.
        let mut acc = vals[0];
        for &v in &vals[1..] {
            let s = g.add(
                Op::Int32(ir::Int32Op::Xor),
                vec![acc, v],
                Repr::Int32,
                Trust::Proven,
                b,
                0,
            );
            g.blocks[0].nodes.push(s);
            acc = s;
        }
        let a = allocate(&g).expect("alloc");
        assert!(a.spilled >= 20 - REGS.len(), "{a:?}");
        // The first 20 values are all live at the first xor: their registers differ.
        let regs: Vec<u8> = vals
            .iter()
            .filter_map(|&v| match a.of(v) {
                Location::Reg(r) => Some(r),
                _ => None,
            })
            .collect();
        let mut sorted = regs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), regs.len(), "{regs:?}");
        assert_eq!(a.of(p), Location::Frame(ep9_vm::layout::fp_param(0) as i32));
        verify(&g, &a).expect("verify");
    }
}
