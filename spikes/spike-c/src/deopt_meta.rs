//! The per-exit deopt metadata (SPIKEC-4; protocol §6.2, C5b; H29-Q1, Q2, Q5), as a builder of
//! the H29 byte form that the VM reads (M5-P1 SPIKEC-5, V10-D30: the form and its reader are
//! `crates/ep9_vm/src/deopt.rs`). The emitter makes one record per exit stub (one per guard,
//! and one per [`Cause`] of a shape guard, MC-w21-5) from the guard's frame state and the
//! register allocation ([`crate::emit_a64::emit`]); each record names its cause as the exit
//! class and the reason of the H29 form (V11-D13, V11-D14; [`Cause::class`],
//! [`Cause::reason`]). The install
//! hands the bytes to `ep9_vm::NativeCx::install_b_with_deopt`, whose reader refuses anything
//! the form does not allow (`ep9_vm::NativeCx::check_deopt_meta` reads them with no install).
//!
//! ```text
//!  table   := uleb records, uleb consts, const* (u64 LE: an immediate Value), record*
//!  record  := u8 header     bits 0-3 record tag (0 exit), bits 4-7 exit class (0 speculation,
//!                           2 not filled: [`Cause::class`])
//!             uleb pc       native offset of the exit stub from the code start
//!             uleb bytecode the bytecode offset T0 resumes at
//!             uleb counter  index into the code's RW exit-counter array (the stub index)
//!             uleb feedback the guard's feedback slot + 1 (a shape guard's IC site), 0 = none
//!             uleb reason   the cause: 0 unknown (a guard with one check), 1-5 the causes of
//!                           a shape guard ([`Cause::reason`])
//!             uleb inline   0 (no inlining)
//!             uleb n, value* every live VM register and the accumulator at the exit
//!  value   := u8 tag        bits 0-3 location (0 reg, 1 stack, 2 const-index, 3 VM frame
//!                           slot), bits 4-6 repr (Tagged, Int32, Bool, Float64), bit 7: the
//!                           target is the accumulator
//!             [target]      the VM register (its bytecode operand; ZigZag + uleb), absent
//!                           for the acc
//!             payload       register number (uleb), stack byte offset from x29 (uleb),
//!                           const index (uleb) or VM frame byte offset from fp (ZigZag + uleb)
//! ```
//!
//! A signed field is ZigZag + uleb, `uleb((v << 1) ^ (v >> 63))` ([`zigzag`]), not SLEB128
//! (H29-Q2).
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
    pub const RECORD_CALL: u8 = 2;
    pub const CLASS_SHIFT: u32 = 4;
    pub const CLASS_SPECULATION: u8 = 0;
    pub const CLASS_ENVIRONMENT: u8 = 1;
    pub const CLASS_NOT_FILLED: u8 = 2;
    pub const REASON_UNKNOWN: u32 = 0;
    pub const REASON_SHAPE_DIFFERS: u32 = 1;
    pub const REASON_NOT_CELL: u32 = 2;
    pub const REASON_NO_VECTOR: u32 = 3;
    pub const REASON_UNFILLED: u32 = 4;
    pub const REASON_MEGAMORPHIC: u32 = 5;
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

/// Why an exit was taken (MODES-CHECK-w21 MC-w21-5): a guard with more than one check gets an
/// exit stub, a record and a counter per cause, so the VM's exit counters count its exits by
/// cause. Since M6-P0 the record carries it (V11-D13, V11-D14): its reason ([`Cause::reason`])
/// and its exit class ([`Cause::class`]: "not filled" for [`Cause::not_filled`], else
/// speculation). [`ExitRecord::guard`] is still not in the bytes: the spike's reports name it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cause {
    /// The guard's one check failed (every guard but `CheckShape`).
    #[default]
    Check,
    /// `CheckShape`: the IC entry holds a case, but not the guard's (another shape in a
    /// monomorphic entry, no POLY_IC case for the shape, or another handler): the fact the
    /// code was compiled from does not hold. The only cause that means a wrong profile.
    Differs,
    /// `CheckShape`: the receiver is not a cell (or is the zero word).
    NotCell,
    /// `CheckShape`: the function has no feedback vector (`FunctionInfo.feedback` is null).
    NoVector,
    /// `CheckShape`: the IC entry's key half is undefined: T0 has not filled it yet, or its
    /// weak key died (feedback.rs: the key half is cleared to undefined).
    Unfilled,
    /// `CheckShape`: the entry is megamorphic (the hole key, a value that is no POLY_IC cell).
    Megamorphic,
}

impl Cause {
    /// The causes of a `CheckShape` guard, in the order of its exit stubs.
    pub const SHAPE: [Cause; 5] = [
        Cause::Differs,
        Cause::NotCell,
        Cause::NoVector,
        Cause::Unfilled,
        Cause::Megamorphic,
    ];

    /// The cause's name in the reports.
    pub fn name(self) -> &'static str {
        match self {
            Cause::Check => "check",
            Cause::Differs => "differs",
            Cause::NotCell => "not-cell",
            Cause::NoVector => "no-vector",
            Cause::Unfilled => "unfilled",
            Cause::Megamorphic => "megamorphic",
        }
    }

    /// True for a cause that says the feedback the guard reads is not there yet (no vector, an
    /// entry T0 has not filled), not that a fact is wrong: the start-up exits of MC-w21-5.
    pub fn not_filled(self) -> bool {
        matches!(self, Cause::NoVector | Cause::Unfilled)
    }

    /// The H29 exit class of the cause's record (header bits 4-7): 2 "not filled" for a
    /// [`Cause::not_filled`] cause, outside the H32 site and function budgets (V11-D14), else 0
    /// speculation.
    pub fn class(self) -> u8 {
        if self.not_filled() {
            form::CLASS_NOT_FILLED
        } else {
            form::CLASS_SPECULATION
        }
    }

    /// The H29 reason of the cause's record (V11-D13): 0 unknown for [`Cause::Check`] (the
    /// guard has one check), 1-5 for the shape-guard causes in [`Cause::SHAPE`] order.
    pub fn reason(self) -> u32 {
        match self {
            Cause::Check => form::REASON_UNKNOWN,
            Cause::Differs => form::REASON_SHAPE_DIFFERS,
            Cause::NotCell => form::REASON_NOT_CELL,
            Cause::NoVector => form::REASON_NO_VECTOR,
            Cause::Unfilled => form::REASON_UNFILLED,
            Cause::Megamorphic => form::REASON_MEGAMORPHIC,
        }
    }
}

/// The metadata of one guard exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitRecord {
    /// Native offset of the exit stub from the code start.
    pub pc: u32,
    /// The guard (not in the bytes: the spike's reports name it).
    pub guard: u32,
    /// Why this exit is taken: the record's exit class and reason ([`Cause`]).
    pub cause: Cause,
    /// The bytecode offset T0 resumes at.
    pub bytecode: u32,
    /// Index of the exit's counter.
    pub counter: u32,
    /// The guard's feedback slot (a shape guard's or a CheckArray's IC site), if it has one.
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
    /// SPIKEC-10 (the lazy-deopt probe, H29-Q4): one call record per call site (H29 record
    /// tag 2, exit class environment), after the exits (their stubs follow the exit stubs,
    /// so the pcs stay sorted; `counter` = the exits' count + the index).
    pub calls: Vec<CallRecord>,
}

/// The call record of one call site (SPIKEC-10): where a lazy deopt of the call lands. A
/// swap writes `stub` (the code's offset of the site's lazy exit) over the return address
/// `ret` of the call's frame; the lazy exit restores what the call saved and takes the
/// shared exit with the record's pc, as an exit stub does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
    /// Native offset of the lazy exit's exit instruction (what the shared exit is given).
    pub pc: u32,
    /// Native offset of the call's return address (the instruction after the `blr`).
    pub ret: u32,
    /// Native offset of the lazy exit's first instruction (the swap target).
    pub stub: u32,
    /// The bytecode offset T0 resumes at: the instruction after the call.
    pub bytecode: u32,
    /// Index of the record's counter.
    pub counter: u32,
    /// Every live VM register after the call, then the accumulator: the call's result in
    /// x0 (Tagged).
    pub values: Vec<ExitValue>,
}

/// A call site of the emitter (SPIKEC-10): the call node, the native offsets of its return
/// address, of its lazy exit's first instruction and of that exit's exit instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LazySite {
    pub node: NodeId,
    pub ret: u32,
    pub stub: u32,
    pub pc: u32,
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

/// A signed field: ZigZag, then uleb (not SLEB128; H29-Q2).
fn zigzag(out: &mut Vec<u8>, v: i64) {
    uleb(out, ((v << 1) ^ (v >> 63)) as u64);
}

impl ExitTable {
    /// The records of the exit stubs in `stubs` (guard id, native offset of the stub, cause; in
    /// stub order) from their guards' frame states in `g` and the locations in `a`. Refuses a
    /// frame-state value with no readable place, or a constant that is a cell.
    pub fn build(
        g: &Graph,
        a: &Allocation,
        stubs: &[(u32, u32, Cause)],
        lazy: &[LazySite],
    ) -> Result<ExitTable, String> {
        let mut t = ExitTable::default();
        for (counter, &(id, pc, cause)) in stubs.iter().enumerate() {
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
                _ => g.sites.iter().find(|s| s.0 == n).map(|s| s.1),
            };
            t.exits.push(ExitRecord {
                pc,
                guard: id,
                cause,
                bytecode: guard.state.offset,
                counter: counter as u32,
                feedback,
                values,
            });
        }
        for (k, site) in lazy.iter().enumerate() {
            let n = site.node;
            let at = format!("{}: call n{} (lazy)", g.name, n.0);
            let state = g
                .node(n)
                .lazy
                .as_ref()
                .ok_or(format!("{at}: no lazy state"))?;
            let mut values = Vec::new();
            for &(r, v) in &state.registers {
                let (repr, loc) = t.place(g, a, v).map_err(|e| format!("{at}: {e}"))?;
                values.push(ExitValue {
                    target: Some(r),
                    repr,
                    loc,
                });
            }
            values.push(ExitValue {
                target: None,
                repr: ValueRepr::Tagged,
                loc: ValueLoc::Reg(0),
            });
            t.calls.push(CallRecord {
                pc: site.pc,
                ret: site.ret,
                stub: site.stub,
                bytecode: state.offset,
                counter: (stubs.len() + k) as u32,
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
                    // A pool double (SPIKEC-9; never an int32 but -0, `build`): the double
                    // Value `Value::number` gives it, NaN purified.
                    Op::ConstFloat64(b) => crate::vm_layout::number_bits(f64::from_bits(b)),
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
        uleb(&mut out, (self.exits.len() + self.calls.len()) as u64);
        uleb(&mut out, self.consts.len() as u64);
        for &c in &self.consts {
            out.extend_from_slice(&c.bits().to_le_bytes());
        }
        // An eager guard exit: record tag exit, the cause's exit class and reason. A call
        // record (SPIKEC-10): tag call, class environment (a lazy deopt from an outside
        // event), no feedback slot, reason 0.
        let exits = self.exits.iter().map(|e| {
            let header = form::RECORD_EXIT | e.cause.class() << form::CLASS_SHIFT;
            let feedback = e.feedback.map_or(0, |s| s + 1);
            let fields = [e.pc, e.bytecode, e.counter, feedback, e.cause.reason(), 0];
            (header, fields, &e.values)
        });
        let calls = self.calls.iter().map(|c| {
            let header = form::RECORD_CALL | form::CLASS_ENVIRONMENT << form::CLASS_SHIFT;
            (header, [c.pc, c.bytecode, c.counter, 0, 0, 0], &c.values)
        });
        for (header, fields, values) in exits.chain(calls) {
            out.push(header);
            for f in fields {
                uleb(&mut out, u64::from(f));
            }
            uleb(&mut out, values.len() as u64);
            for v in values {
                let (loc, payload) = match v.loc {
                    ValueLoc::Reg(r) => (form::LOC_REG, i64::from(r)),
                    ValueLoc::Stack(k) => (form::LOC_STACK, i64::from(stack_offset(k))),
                    ValueLoc::Const(i) => (form::LOC_CONST, i64::from(i)),
                    ValueLoc::Frame(o) => (form::LOC_FRAME, i64::from(o)),
                };
                let acc = if v.target.is_none() { form::ACC } else { 0 };
                out.push(loc | (v.repr as u8) << form::REPR_SHIFT | acc);
                if let Some(r) = v.target {
                    zigzag(&mut out, i64::from(r.to_operand()));
                }
                if loc == form::LOC_FRAME {
                    zigzag(&mut out, payload);
                } else {
                    uleb(&mut out, payload as u64);
                }
            }
        }
        out
    }

    /// The values of every record by location kind (the H29-Q2 report): `reg n, stack n, const
    /// n, frame n`.
    pub fn locations_text(&self) -> String {
        let mut n = [0usize; 4];
        for v in self.exits.iter().flat_map(|e| &e.values) {
            n[match v.loc {
                ValueLoc::Reg(_) => 0,
                ValueLoc::Stack(_) => 1,
                ValueLoc::Const(_) => 2,
                ValueLoc::Frame(_) => 3,
            }] += 1;
        }
        format!(
            "reg {}, stack {}, const {}, frame {}",
            n[0], n[1], n[2], n[3]
        )
    }

    /// One report line: exits, bytes, bytes per exit, values per exit.
    pub fn line(&self) -> String {
        let bytes = self.encode().len();
        let n = self.exits.len().max(1) as f64;
        let values: usize = self.exits.iter().map(|e| e.values.len()).sum();
        let calls = if self.calls.is_empty() {
            String::new()
        } else {
            format!(" + {} call records", self.calls.len())
        };
        format!(
            "exit metadata: {} records ({} guards){calls}, {} consts, {bytes} bytes ({:.1} per \
             exit, {:.1} values per exit; values by location: {}); offsets and indexes only",
            self.exits.len(),
            self.exits
                .iter()
                .map(|e| e.guard)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            self.consts.len(),
            bytes as f64 / n,
            values as f64 / n,
            self.locations_text(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ExitTable {
        ExitTable {
            calls: Vec::new(),
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
                    cause: Cause::Unfilled,
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
                    cause: Cause::Check,
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
    /// accumulator, a parameter, a local, a feedback slot, a not-filled class and a reason. The
    /// bytes are the H29 form byte for byte (the head of the record of guard 3, all of guard 9).
    #[test]
    fn the_vm_reads_the_built_table() {
        let t = sample();
        let b = t.encode();
        assert_eq!(ep9_vm::NativeCx::check_deopt_meta(&b), Ok(2));
        // Record 1 (after the counts and 2 consts, byte 18): header 0x20 (exit, class 2 not
        // filled), pc 2100 (uleb b4 10), bytecode 57, counter 0, feedback slot 2 + 1, reason 4
        // (unfilled), no inline frame, 3 values.
        let head = [0x20, 0xB4, 0x10, 0x39, 0x00, 0x03, 0x04, 0x00, 0x03];
        assert_eq!(b[18..27], head, "{b:02x?}");
        // Record 2: header 0 (exit, speculation), pc 2112 (uleb c0 10), bytecode 0, counter 1,
        // no feedback, reason 0 (unknown: a guard with one check), no inline frame, 1 value:
        // tag 0x02 (const-index, Tagged, a register target), target r0 (ZigZag 0), const 1.
        let tail = [
            0x00, 0xC0, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x01,
        ];
        assert!(b.ends_with(&tail), "{b:02x?}");
        // A spill slot is a byte offset from x29: slot 3 is x29 + 120.
        assert_eq!(stack_offset(3), 120);
    }

    /// One record per [`Cause`]: the class and reason bytes the builder writes (V11-D13,
    /// V11-D14), which the VM's reader takes; a reason or class the VM does not know is
    /// refused. The values are the VM's own `deopt::form` constants, read from its source text
    /// (they are crate-private), so a renumbering on either side fails here.
    #[test]
    fn each_cause_writes_its_class_and_reason() {
        let causes = [
            Cause::Check,
            Cause::Differs,
            Cause::NotCell,
            Cause::NoVector,
            Cause::Unfilled,
            Cause::Megamorphic,
        ];
        let t = ExitTable {
            calls: Vec::new(),
            consts: vec![],
            exits: (0u32..)
                .zip(causes)
                .map(|(i, cause)| ExitRecord {
                    pc: 4 * i,
                    guard: 0,
                    cause,
                    bytecode: 0,
                    counter: i,
                    feedback: None,
                    values: vec![],
                })
                .collect(),
        };
        let b = t.encode();
        assert_eq!(ep9_vm::NativeCx::check_deopt_meta(&b), Ok(6));
        // Each record: header, pc, bytecode, counter, feedback, reason, inline, n (8 bytes,
        // every field below 128), after the two counts.
        let got: Vec<(u8, u8)> = b[2..].chunks(8).map(|r| (r[0], r[5])).collect();
        assert_eq!(
            got,
            [(0, 0), (0, 1), (0, 2), (0x20, 3), (0x20, 4), (0, 5)],
            "{b:02x?}"
        );
        const DEOPT_RS: &str = include_str!("../../../crates/ep9_vm/src/deopt.rs");
        let vm = |name: &str, ty: &str| -> u32 {
            let prefix = format!("pub const {name}: {ty} =");
            let l = DEOPT_RS
                .lines()
                .find_map(|l| l.trim().strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("no {prefix} in deopt.rs"));
            l.trim().trim_end_matches(';').parse().expect("a number")
        };
        assert_eq!(vm("CLASS_SHIFT", "u32"), form::CLASS_SHIFT);
        assert_eq!(
            vm("CLASS_SPECULATION", "u8"),
            u32::from(form::CLASS_SPECULATION)
        );
        assert_eq!(
            vm("CLASS_NOT_FILLED", "u8"),
            u32::from(form::CLASS_NOT_FILLED)
        );
        for (name, cause) in [
            ("REASON_UNKNOWN", Cause::Check),
            ("REASON_SHAPE_DIFFERS", Cause::Differs),
            ("REASON_NOT_CELL", Cause::NotCell),
            ("REASON_NO_VECTOR", Cause::NoVector),
            ("REASON_UNFILLED", Cause::Unfilled),
            ("REASON_MEGAMORPHIC", Cause::Megamorphic),
        ] {
            assert_eq!(vm(name, "u32"), cause.reason(), "{name}");
        }
        // The next reason and class 15 are refused (the VM knows reasons 0-5, classes 0-2).
        let mut bad = b.clone();
        bad[2 + 8 * 5 + 5] = 6;
        let e = ep9_vm::NativeCx::check_deopt_meta(&bad).unwrap_err();
        assert!(e.contains("unknown exit reason 6"), "{e}");
        let mut bad = b.clone();
        bad[2] = 0xF0;
        let e = ep9_vm::NativeCx::check_deopt_meta(&bad).unwrap_err();
        assert!(e.contains("unknown exit class 15"), "{e}");
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

    /// The signed fields (a value's VM register target and a VM frame slot's byte offset from
    /// fp) are ZigZag-mapped, `(v << 1) ^ (v >> 63)`, then written as a uleb: not SLEB128
    /// (two's complement, as in DWARF CFI). The fixed bytes of both forms differ for every
    /// value but 0, so this pins the form (Codex r1 spikec8, H29-Q2).
    #[test]
    fn signed_fields_are_zigzag_uleb_not_sleb128() {
        // (value, ZigZag + uleb: what `zigzag` writes, SLEB128: what it does not write)
        let cases: [(i64, &[u8], &[u8]); 11] = [
            (0, &[0x00], &[0x00]),
            (-1, &[0x01], &[0x7F]),
            (1, &[0x02], &[0x01]),
            (-64, &[0x7F], &[0x40]),
            (63, &[0x7E], &[0x3F]),
            (64, &[0x80, 0x01], &[0xC0, 0x00]),
            (-65, &[0x81, 0x01], &[0xBF, 0x7F]),
            (-8, &[0x0F], &[0x78]),
            (-123_456, &[0xFF, 0x88, 0x0F], &[0xC0, 0xBB, 0x78]),
            (
                i64::from(i32::MIN),
                &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
                &[0x80, 0x80, 0x80, 0x80, 0x78],
            ),
            (
                i64::from(i32::MAX),
                &[0xFE, 0xFF, 0xFF, 0xFF, 0x0F],
                &[0xFF, 0xFF, 0xFF, 0xFF, 0x07],
            ),
        ];
        for (v, want, sleb128) in cases {
            let mut out = Vec::new();
            zigzag(&mut out, v);
            assert_eq!(out, want, "{v}");
            assert_eq!(out == sleb128, v == 0, "{v}");
        }
    }

    /// The VM's reader (`ep9_vm` `deopt.rs` `Reader::zigzag32`) reads the same ZigZag + uleb
    /// bytes back: a word offset is taken, and a non-word offset is refused with the exact
    /// value it read. The SLEB128 bytes of the same offset read as another value.
    #[test]
    fn the_vm_reads_the_zigzag_bytes_back() {
        // One exit record with one value: the accumulator in a VM frame slot at `o`.
        let table = |o: i32| ExitTable {
            calls: Vec::new(),
            consts: vec![],
            exits: vec![ExitRecord {
                pc: 0,
                guard: 0,
                cause: Cause::Check,
                bytecode: 0,
                counter: 0,
                feedback: None,
                values: vec![ExitValue {
                    target: None,
                    repr: ValueRepr::Tagged,
                    loc: ValueLoc::Frame(o),
                }],
            }],
        };
        // records 1, consts 0, header 0, pc 0, bytecode 0, counter 0, feedback 0, reason 0,
        // inline 0, values 1, tag 0x83 (VM frame slot, Tagged, the accumulator), offset -8 =
        // 0x0F.
        let b = table(-8).encode();
        assert_eq!(
            b,
            [
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x83, 0x0F
            ]
        );
        let check = ep9_vm::NativeCx::check_deopt_meta;
        for o in [-8, 8, -64, 64, -4096, i32::MIN] {
            assert_eq!(check(&table(o).encode()), Ok(1), "{o}");
        }
        for o in [-12, -65, 63, -4100, i32::MAX] {
            let e = check(&table(o).encode()).unwrap_err();
            assert!(
                e.contains(&format!("byte offset {o} is not a word")),
                "{o}: {e}"
            );
        }
        // SLEB128 bytes of -8 (0x78) and 8 (0x08): the reader reads 60 and 4.
        for (sleb128, read) in [(0x78u8, 60), (0x08, 4)] {
            let mut bad = b.clone();
            *bad.last_mut().unwrap() = sleb128;
            let e = check(&bad).unwrap_err();
            assert!(
                e.contains(&format!("byte offset {read} is not a word")),
                "{e}"
            );
        }
    }
}
