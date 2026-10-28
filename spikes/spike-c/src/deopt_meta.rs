//! The per-exit deopt metadata (SPIKEC-4; protocol §6.2, C5b; H29-Q1, Q2, Q5), as a builder of
//! the H29 byte form that the VM reads (M5-P1 SPIKEC-5, V10-D30: the form and its reader are
//! `crates/ep9_vm/src/deopt.rs`). The emitter makes one record per guard exit from the
//! guard's frame state and the register allocation ([`crate::emit_a64::emit`]); the install
//! hands the bytes to `ep9_vm::NativeCx::install_b_with_deopt`, whose reader refuses anything
//! the form does not allow (`ep9_vm::NativeCx::check_deopt_meta` reads them with no install).
//!
//! ```text
//!  table   := uleb records, uleb consts, const* (u64 LE: an immediate Value), record*
//!  record  := u8 header     bits 0-3 record tag (0 exit), bit 4 exit class (0 speculation)
//!             uleb pc       native offset of the exit stub from the code start
//!             uleb bytecode the bytecode offset T0 resumes at
//!             uleb counter  index into the code's RW exit-counter array (the stub index)
//!             uleb feedback the guard's feedback slot + 1 (a shape guard's IC site), 0 = none
//!             uleb inline   0 (no inlining)
//!             uleb n, value* every live VM register and the accumulator at the exit
//!  value   := u8 tag        bits 0-3 location (0 reg, 1 stack, 2 const-index, 3 VM frame
//!                           slot), bits 4-6 repr (Tagged, Int32, Bool, Float64), bit 7: the
//!                           target is the accumulator
//!             [sleb target] the VM register (its bytecode operand), absent for the acc
//!             payload        register number (uleb), stack byte offset from x29 (uleb),
//!                           const index (uleb) or VM frame byte offset from fp (sleb)
//! ```
//!
//! **Position independent.** Every field is an offset or an index: the pc from the code
//! start, the bytecode offset from the function's bytecode start, a native stack word from
//! the compiled frame's x29, a VM frame slot from fp, a const by index into the table's own
//! list of immediate Values (no cell: H30 has no raw heap constant, and [`ExitTable::build`]
//! refuses one). So code + table can move together, and two isolates that compile the same
//! function from the same facts make byte-equal tables (test
//! `metadata_is_the_same_in_two_isolates`).
//!
//! **What the deopt entry does with a record** (`ep9_vm` `deopt.rs`): the shared exit passes
//! the exit stub's pc offset ([`crate::emit_a64`]); the entry finds the record by it, boxes
//! each value by its repr, writes it to its VM register or the accumulator, adds 1 to the
//! counter and resumes T0 at the bytecode offset.

use ep9_bytecode::Register;

use crate::ir::{Graph, NodeId, Op, Repr};
use crate::regalloc::{Allocation, Location};

/// The H29 byte form's tag values (the VM's `deopt::form`; the VM's reader is the check).
mod form {
    pub const RECORD_EXIT: u8 = 0;
    pub const LOC_REG: u8 = 0;
    pub const LOC_STACK: u8 = 1;
    pub const LOC_CONST: u8 = 2;
    pub const LOC_FRAME: u8 = 3;
    pub const REPR_SHIFT: u32 = 4;
    pub const ACC: u8 = 0x80;
}

/// The machine form of a value at an exit (the H29 repr values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueRepr {
    Tagged = 0,
    Int32 = 1,
    Bool = 2,
    Float64 = 3,
}

/// Where a value is at an exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueLoc {
    /// A machine register `x<n>`.
    Reg(u8),
    /// Spill slot `k` of the compiled frame: the word at x29 + [`stack_offset`]`(k)`.
    Stack(u32),
    /// The VM frame slot at this byte offset from fp (the receiver, a parameter).
    Frame(i32),
    /// Entry `i` of the table's const list.
    Const(u32),
}

/// One value of an exit: the VM register it goes to (`None`: the accumulator), its form
/// and its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitValue {
    pub target: Option<Register>,
    pub repr: ValueRepr,
    pub loc: ValueLoc,
}

/// The metadata of one guard exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitRecord {
    /// Native offset of the exit stub from the code start.
    pub pc: u32,
    /// The guard (not in the bytes: the spike's reports name it).
    pub guard: u32,
    /// The bytecode offset T0 resumes at.
    pub bytecode: u32,
    /// Index of the exit's counter.
    pub counter: u32,
    /// The guard's feedback slot (a shape guard's IC site), if it has one.
    pub feedback: Option<u32>,
    pub values: Vec<ExitValue>,
}

/// An immediate Value (an int32, a double, undefined, null, a boolean): never a cell, so a
/// table of them holds nothing the GC must see (plan rule 8; H30: no raw heap constant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Immediate(u64);

impl Immediate {
    /// `bits` as an immediate, or `None` for a cell.
    pub fn new(bits: u64) -> Option<Immediate> {
        (!crate::vm_layout::is_cell(bits)).then_some(Immediate(bits))
    }

    /// The Value bits.
    pub fn bits(self) -> u64 {
        self.0
    }
}

/// The exit metadata of one compiled function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitTable {
    /// Immediate Values the records name by index.
    pub consts: Vec<Immediate>,
    /// One record per exit, in stub order (`counter` = the index), so sorted by pc.
    pub exits: Vec<ExitRecord>,
}

/// The byte offset from x29 of spill slot `k` (the emitter's frame: spill slots from
/// sp + 96, and x29 = sp after the prologue).
pub fn stack_offset(k: u32) -> u32 {
    crate::emit_a64::spill_slot_offset(k)
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, v: i64) {
    uleb(out, ((v << 1) ^ (v >> 63)) as u64);
}

impl ExitTable {
    /// The records of the guards in `stubs` (guard id, native offset of its stub, in stub
    /// order) from their frame states in `g` and the locations in `a`. Refuses a frame-state
    /// value with no readable place, or a constant that is a cell.
    pub fn build(g: &Graph, a: &Allocation, stubs: &[(u32, u32)]) -> Result<ExitTable, String> {
        let mut t = ExitTable::default();
        for (counter, &(id, pc)) in stubs.iter().enumerate() {
            let (n, guard) = g
                .guard_nodes()
                .find(|(_, gd)| gd.id == id)
                .ok_or_else(|| format!("no live guard g{id}"))?;
            let at = format!("{}: g{id} (n{})", g.name, n.0);
            let mut values = Vec::new();
            let state = guard
                .state
                .registers
                .iter()
                .map(|&(r, v)| (Some(r), v))
                .chain(guard.state.acc.map(|v| (None, v)));
            for (target, v) in state {
                let (repr, loc) = t.place(g, a, v).map_err(|e| format!("{at}: {e}"))?;
                values.push(ExitValue { target, repr, loc });
            }
            let feedback = match g.node(n).op {
                Op::CheckShape { site, .. } => Some(site),
                _ => None,
            };
            t.exits.push(ExitRecord {
                pc,
                guard: id,
                bytecode: guard.state.offset,
                counter: counter as u32,
                feedback,
                values,
            });
        }
        Ok(t)
    }

    /// The form and place of the value `v` at an exit.
    fn place(
        &mut self,
        g: &Graph,
        a: &Allocation,
        v: NodeId,
    ) -> Result<(ValueRepr, ValueLoc), String> {
        let node = g.node(v);
        let repr = match node.repr {
            Repr::Tagged => ValueRepr::Tagged,
            Repr::Int32 => ValueRepr::Int32,
            Repr::Bool => ValueRepr::Bool,
            Repr::Float64 => ValueRepr::Float64,
            Repr::None => return Err(format!("n{} has no value", v.0)),
        };
        Ok(match a.of(v) {
            Location::Reg(r) => (repr, ValueLoc::Reg(r)),
            Location::Stack(k) => (repr, ValueLoc::Stack(k)),
            Location::Frame(off) => (repr, ValueLoc::Frame(off)),
            Location::Const => {
                // A constant is kept as the Value T0 sees.
                let bits = match node.op {
                    Op::Const(b) => b,
                    Op::ConstInt32(i) => crate::vm_layout::int32_bits(i),
                    ref op => return Err(format!("n{} is a constant op {op:?}", v.0)),
                };
                let c = Immediate::new(bits).ok_or(format!("n{} is a heap constant", v.0))?;
                let i = match self.consts.iter().position(|&x| x == c) {
                    Some(i) => i,
                    None => {
                        self.consts.push(c);
                        self.consts.len() - 1
                    }
                };
                (ValueRepr::Tagged, ValueLoc::Const(i as u32))
            }
            l @ (Location::None | Location::Fused) => {
                return Err(format!("n{} has no readable place ({l:?})", v.0));
            }
        })
    }

    /// The H29 byte form (module docs).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        uleb(&mut out, self.exits.len() as u64);
        uleb(&mut out, self.consts.len() as u64);
        for &c in &self.consts {
            out.extend_from_slice(&c.bits().to_le_bytes());
        }
        for e in &self.exits {
            // An eager guard exit: record tag exit, exit class speculation (bit 4 clear).
            out.push(form::RECORD_EXIT);
            let feedback = e.feedback.map_or(0, |s| s + 1);
            for f in [e.pc, e.bytecode, e.counter, feedback, 0] {
                uleb(&mut out, u64::from(f));
            }
            uleb(&mut out, e.values.len() as u64);
            for v in &e.values {
                let (loc, payload) = match v.loc {
                    ValueLoc::Reg(r) => (form::LOC_REG, i64::from(r)),
                    ValueLoc::Stack(k) => (form::LOC_STACK, i64::from(stack_offset(k))),
                    ValueLoc::Const(i) => (form::LOC_CONST, i64::from(i)),
                    ValueLoc::Frame(o) => (form::LOC_FRAME, i64::from(o)),
                };
                let acc = if v.target.is_none() { form::ACC } else { 0 };
                out.push(loc | (v.repr as u8) << form::REPR_SHIFT | acc);
                if let Some(r) = v.target {
                    sleb(&mut out, i64::from(r.to_operand()));
                }
                if loc == form::LOC_FRAME {
                    sleb(&mut out, payload);
                } else {
                    uleb(&mut out, payload as u64);
                }
            }
        }
        out
    }

    /// One report line: exits, bytes, bytes per exit, values per exit.
    pub fn line(&self) -> String {
        let bytes = self.encode().len();
        let n = self.exits.len().max(1) as f64;
        let values: usize = self.exits.iter().map(|e| e.values.len()).sum();
        format!(
            "exit metadata: {} records, {} consts, {bytes} bytes ({:.1} per exit, {:.1} values per \
             exit); offsets and indexes only",
            self.exits.len(),
            self.consts.len(),
            bytes as f64 / n,
            values as f64 / n,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ExitTable {
        ExitTable {
            consts: [
                crate::vm_layout::int32_bits(7),
                ep9_value::encoding::VALUE_UNDEFINED,
            ]
            .map(|b| Immediate::new(b).expect("immediate"))
            .to_vec(),
            exits: vec![
                ExitRecord {
                    pc: 2100,
                    guard: 3,
                    bytecode: 57,
                    counter: 0,
                    feedback: Some(2),
                    values: vec![
                        ExitValue {
                            target: Some(Register::local(4)),
                            repr: ValueRepr::Int32,
                            loc: ValueLoc::Reg(19),
                        },
                        ExitValue {
                            target: Some(Register::parameter(1)),
                            repr: ValueRepr::Tagged,
                            loc: ValueLoc::Frame(-88),
                        },
                        ExitValue {
                            target: None,
                            repr: ValueRepr::Float64,
                            loc: ValueLoc::Stack(3),
                        },
                    ],
                },
                ExitRecord {
                    pc: 2112,
                    guard: 9,
                    bytecode: 0,
                    counter: 1,
                    feedback: None,
                    values: vec![ExitValue {
                        target: Some(Register::local(0)),
                        repr: ValueRepr::Tagged,
                        loc: ValueLoc::Const(1),
                    }],
                },
            ],
        }
    }

    /// The VM's reader takes what this builder writes: every location kind, every repr, the
    /// accumulator, a parameter, a local and a feedback slot. The bytes are the H29 form
    /// byte for byte (the record of guard 9 below).
    #[test]
    fn the_vm_reads_the_built_table() {
        let t = sample();
        let b = t.encode();
        assert_eq!(ep9_vm::NativeCx::check_deopt_meta(&b), Ok(2));
        // Record 2: header 0 (exit, speculation), pc 2112 (uleb c0 10), bytecode 0, counter 1,
        // no feedback, no inline frame, 1 value: tag 0x02 (const-index, Tagged, a register
        // target), target r0 (sleb 0), const 1.
        let tail = [
            0x00, 0xC0, 0x10, 0x00, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x01,
        ];
        assert!(b.ends_with(&tail), "{b:02x?}");
        // A spill slot is a byte offset from x29: slot 3 is x29 + 120.
        assert_eq!(stack_offset(3), 120);
    }

    /// What the VM's reader refuses, the builder never writes: a cut table, a const index past
    /// the list, a cell in the const list, two records at one pc.
    #[test]
    fn the_vm_refuses_a_broken_table() {
        let t = sample();
        let b = t.encode();
        assert!(ep9_vm::NativeCx::check_deopt_meta(&b[..b.len() - 1]).is_err());
        let mut bad = t.clone();
        bad.exits[1].values[0].loc = ValueLoc::Const(5);
        let e = ep9_vm::NativeCx::check_deopt_meta(&bad.encode()).unwrap_err();
        assert!(e.contains("const 5 of 2"), "{e}");
        let mut pc = t.clone();
        pc.exits[1].pc = pc.exits[0].pc;
        let e = ep9_vm::NativeCx::check_deopt_meta(&pc.encode()).unwrap_err();
        assert!(e.contains("sorted by pc"), "{e}");
        // A cell address in the const list is refused on both sides.
        assert!(Immediate::new(0x1000).is_none());
        let mut cell = t.encode();
        cell[2..10].copy_from_slice(&0x1000u64.to_le_bytes());
        let e = ep9_vm::NativeCx::check_deopt_meta(&cell).unwrap_err();
        assert!(e.contains("not an immediate Value"), "{e}");
    }
}
