//! The per-exit deopt metadata form (SPIKEC-4; protocol §6.2, C5b; H29-Q1, Q2, Q5). No
//! runtime reads it yet: the month-5 deopt entry will. The emitter makes one record per
//! guard exit from the guard's frame state and the register allocation
//! ([`crate::emit_a64::emit`]); `spike-c compile` prints its size.
//!
//! ```text
//!  table   := uleb exits, uleb consts, const* (u64 LE: an immediate Value), record*
//!  record  := uleb pc        native offset of the exit stub from the code start
//!             uleb guard     the guard id (the trap names it)
//!             uleb bytecode  the bytecode offset T0 resumes at
//!             uleb counter   index into the function's RW exit-counter array
//!             uleb n, value* every live VM register and the accumulator at the exit
//!  value   := u8 tag         bits 0-1 repr (Tagged, Int32, Bool, Float64), bits 2-3 kind
//!                            (machine register, native stack slot, VM frame slot, const),
//!                            bit 4: the target is the accumulator
//!             [sleb target]  the VM register (its bytecode operand), absent for the acc
//!             payload        register number, spill slot, frame byte offset (sleb) or
//!                            const index (uleb)
//! ```
//!
//! **Position independent.** Every field is an offset or an index: the pc from the code
//! start, the bytecode offset from the function's bytecode start, a native stack slot from
//! the compiled frame's sp, a VM frame slot from fp, a const by index into the table's own
//! list of immediate Values (no cell: H30 has no raw heap constant, and [`ExitTable::build`]
//! refuses one). So code + table can move together, and two isolates that compile the same
//! function from the same facts make byte-equal tables (test
//! `metadata_is_the_same_in_two_isolates`).
//!
//! **What the deopt entry does with a record** (month 5): find the record of the exit (by pc,
//! or by index for the call-form stubs of [`crate::emit_a64::StubForm::Call`]), box each value
//! by its repr (Int32 and Bool to a Value, Float64 to a number Value), write it to its VM
//! register slot or the accumulator, add 1 to the counter, resume T0 at the bytecode offset.
//! The entry runs from the exit stub, which passes the guard id and offset in x16/x17 (the
//! 12-byte form) or the index in x30 (the call form): it saves x16/x17 (or x30) before any
//! table call or far branch, since a linker veneer or a call may change them (MC-w15-14).

use ep9_bytecode::Register;

use crate::ir::{Graph, NodeId, Op, Repr};
use crate::regalloc::{Allocation, Location};

/// The machine form of a value at an exit.
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
    /// Native stack slot `k` of the compiled frame (sp + 96 + 8k).
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
    pub guard: u32,
    /// The bytecode offset T0 resumes at.
    pub bytecode: u32,
    /// Index of the exit's counter.
    pub counter: u32,
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
    /// One record per exit, in stub order (`counter` = the index).
    pub exits: Vec<ExitRecord>,
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

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let x = *self.b.get(self.at).ok_or("truncated")?;
        self.at += 1;
        Ok(x)
    }

    fn uleb(&mut self) -> Result<u64, String> {
        let (mut v, mut shift) = (0u64, 0u32);
        loop {
            let b = self.byte()?;
            if shift >= 64 {
                return Err("uleb too long".into());
            }
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    fn u32(&mut self) -> Result<u32, String> {
        u32::try_from(self.uleb()?).map_err(|_| "a u32 field is too large".to_string())
    }

    fn sleb(&mut self) -> Result<i64, String> {
        let z = self.uleb()?;
        Ok(((z >> 1) as i64) ^ -((z & 1) as i64))
    }
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
            t.exits.push(ExitRecord {
                pc,
                guard: id,
                bytecode: guard.state.offset,
                counter: counter as u32,
                values,
            });
        }
        Ok(t)
    }

    /// The form and place of the value `v` at an exit.
    fn place(&mut self, g: &Graph, a: &Allocation, v: NodeId) -> Result<(ValueRepr, ValueLoc), String> {
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

    /// The encoded form (module docs).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        uleb(&mut out, self.exits.len() as u64);
        uleb(&mut out, self.consts.len() as u64);
        for &c in &self.consts {
            out.extend_from_slice(&c.bits().to_le_bytes());
        }
        for e in &self.exits {
            for f in [e.pc, e.guard, e.bytecode, e.counter, e.values.len() as u32] {
                uleb(&mut out, u64::from(f));
            }
            for v in &e.values {
                let (kind, payload) = match v.loc {
                    ValueLoc::Reg(r) => (0u8, i64::from(r)),
                    ValueLoc::Stack(k) => (1, i64::from(k)),
                    ValueLoc::Frame(o) => (2, i64::from(o)),
                    ValueLoc::Const(i) => (3, i64::from(i)),
                };
                let acc = if v.target.is_none() { 0x10 } else { 0 };
                out.push(v.repr as u8 | kind << 2 | acc);
                if let Some(r) = v.target {
                    sleb(&mut out, i64::from(r.to_operand()));
                }
                if kind == 2 {
                    sleb(&mut out, payload);
                } else {
                    uleb(&mut out, payload as u64);
                }
            }
        }
        out
    }

    /// Reads an encoded table back.
    pub fn decode(bytes: &[u8]) -> Result<ExitTable, String> {
        let mut r = Reader { b: bytes, at: 0 };
        let n = r.uleb()?;
        let nc = r.uleb()?;
        let mut t = ExitTable::default();
        for _ in 0..nc {
            let s = bytes.get(r.at..r.at + 8).ok_or("truncated const")?;
            let bits = u64::from_le_bytes(s.try_into().expect("8 bytes"));
            t.consts
                .push(Immediate::new(bits).ok_or(format!("const {bits:#x} is a cell"))?);
            r.at += 8;
        }
        for _ in 0..n {
            let (pc, guard, bytecode, counter, nv) =
                (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let mut values = Vec::new();
            for _ in 0..nv {
                let tag = r.byte()?;
                let repr = match tag & 3 {
                    0 => ValueRepr::Tagged,
                    1 => ValueRepr::Int32,
                    2 => ValueRepr::Bool,
                    _ => ValueRepr::Float64,
                };
                let target = if tag & 0x10 != 0 {
                    None
                } else {
                    let op = i32::try_from(r.sleb()?).map_err(|_| "register operand")?;
                    Some(Register::from_operand(op))
                };
                let loc = match (tag >> 2) & 3 {
                    0 => ValueLoc::Reg(u8::try_from(r.uleb()?).map_err(|_| "register")?),
                    1 => ValueLoc::Stack(r.u32()?),
                    2 => ValueLoc::Frame(i32::try_from(r.sleb()?).map_err(|_| "frame offset")?),
                    _ => {
                        let i = r.u32()?;
                        if i as usize >= t.consts.len() {
                            return Err(format!("const {i} of {}", t.consts.len()));
                        }
                        ValueLoc::Const(i)
                    }
                };
                values.push(ExitValue { target, repr, loc });
            }
            t.exits.push(ExitRecord {
                pc,
                guard,
                bytecode,
                counter,
                values,
            });
        }
        if r.at != bytes.len() {
            return Err(format!("{} bytes after the last record", bytes.len() - r.at));
        }
        Ok(t)
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

    #[test]
    fn leb_round_trip() {
        for v in [0u64, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            let mut b = Vec::new();
            uleb(&mut b, v);
            assert_eq!(Reader { b: &b, at: 0 }.uleb(), Ok(v));
        }
        for v in [0i64, -1, 1, -64, 64, -100_000, i64::from(i32::MIN)] {
            let mut b = Vec::new();
            sleb(&mut b, v);
            assert_eq!(Reader { b: &b, at: 0 }.sleb(), Ok(v));
        }
    }

    /// Every location kind, the accumulator, a parameter and a local round-trip; a cut table
    /// or a const index past the list is refused.
    #[test]
    fn encode_decode() {
        let t = ExitTable {
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
                    values: vec![
                        ExitValue {
                            target: Some(Register::local(4)),
                            repr: ValueRepr::Int32,
                            loc: ValueLoc::Reg(19),
                        },
                        ExitValue {
                            target: Some(Register::parameter(1)),
                            repr: ValueRepr::Tagged,
                            loc: ValueLoc::Frame(-40),
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
                    values: vec![ExitValue {
                        target: Some(Register::local(0)),
                        repr: ValueRepr::Tagged,
                        loc: ValueLoc::Const(1),
                    }],
                },
            ],
        };
        let b = t.encode();
        assert_eq!(ExitTable::decode(&b), Ok(t.clone()));
        assert!(ExitTable::decode(&b[..b.len() - 1]).is_err());
        let mut bad = t.clone();
        bad.exits[1].values[0].loc = ValueLoc::Const(5);
        assert!(ExitTable::decode(&bad.encode()).is_err());
        // A cell address in the const list is refused on both sides.
        assert!(Immediate::new(0x1000).is_none());
        let mut cell = t.encode();
        cell[2..10].copy_from_slice(&0x1000u64.to_le_bytes());
        assert!(ExitTable::decode(&cell).is_err());
    }
}
