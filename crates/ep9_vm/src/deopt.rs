// The deopt core is in every build (C-10), but in month 5 its only caller is the RuntimeTable
// glue behind `spike-native` (`native.rs`); the jitless build compiles it and its tests run it.
#![cfg_attr(not(feature = "spike-native"), allow(dead_code))]
//! The deopt core (owner M5-P1 SPIKEC-5, then SPIKEC-6; H29, V10-D29 to V10-D32, SC WI-SC2
//! and WI-SC3): the H29 metadata form and its reader, the frame rebuild, the exit counters
//! and the per-isolate side table. It has no `cfg`: every build has it and the jitless build
//! tests it (C-10, doc 13:371). Only the RuntimeTable glue (`native.rs` `rt_deopt`, entry 10)
//! and the install with metadata stay behind `spike-native` (V10-D29).
//!
//! ```text
//!  guard fails in native code -> exit stub -> shared exit: stores the stub's state first
//!    (x30 of the call form, x16/x17 of the movz form; MC-w15-14), then x0-x30 below sp
//!    -> RuntimeTable::deopt(cx, fp, pc offset of the stub, register dump, x29)
//!  rt_deopt (native.rs) -> SideTable::deopt(code start, pc offset)
//!    -> the exit's Record (found by its pc offset) -> rebuild(): read each value (register
//!       dump, native frame, constant, VM frame slot), box it by its repr (no allocation),
//!       write it to its VM register or the accumulator -> the exit's counter + 1
//!    -> Cx::out_pc = bytecode start + offset, Cx::out_fp = fp: T0 resumes in the same frame
//! ```
//!
//! **The H29 byte form** (V10-D30; the H29 Freeze table lists, H29-Q1, Q2, Q5). A table is
//! the metadata of one code object. Every field is an offset or an index, so code and table
//! can move together (position independent, H29; C5b):
//!
//! ```text
//!  table   := uleb records, uleb consts, const* (u64 LE: an immediate Value, never a cell),
//!             record* (sorted by pc, no two at one pc)
//!  record  := u8 header     bits 0-3 record tag: 0 exit, 1 poll, 2 call (append-only, 16 values)
//!                           bits 4-7 exit class: 0 speculation, 1 environment, 2 not filled
//!                           (append-only, 16 values; H29-Q1 fill, C-04; V11-D14)
//!             uleb pc       offset of the exit stub from the code start
//!             uleb bytecode the bytecode offset T0 resumes at (H2: the only deopt id)
//!             uleb counter  index into the code's RW exit-counter array (H29-Q5; < records)
//!             uleb feedback the guard's feedback slot + 1, 0 = none (a note, MC-w13-7)
//!             uleb reason   why the exit is taken: 0 unknown, 1 shape differs, 2 not a cell,
//!                           3 no vector, 4 unfilled, 5 megamorphic (append-only; V11-D13)
//!             uleb inline   inline frames: 0 (no inline frame kind is built; more is refused)
//!             uleb n, value*
//!  value   := u8 tag        bits 0-3 location: 0 reg, 1 stack, 2 const-index, 3 VM frame slot
//!                           (append-only, 16 values; 3 is the M5-P1 fill, V10-D30)
//!                           bits 4-6 repr: 0 Tagged, 1 Int32, 2 Bool, 3 Float64 (4-7 refused)
//!                           bit 7: the target is the accumulator
//!             [target]      the VM register (its bytecode operand; ZigZag + uleb), absent
//!                           for the accumulator
//!             payload       reg: uleb machine register (< 31, the dump's index)
//!                           stack: uleb byte offset from the exit's native frame pointer (8n)
//!                           const-index: uleb index into the table's consts
//!                           VM frame slot: ZigZag + uleb byte offset from fp (8n; the
//!                           receiver, a parameter or a register of the frame)
//! ```
//!
//! A signed field is ZigZag + uleb: `uleb((v << 1) ^ (v >> 63))`, so -1 is 0x01 and 1 is 0x02
//! (not SLEB128, the two's-complement form of DWARF CFI; H29-Q2).
//!
//! **Exit class and reason** (H29-Q1; V11-D13, V11-D14). The class says what the exit means
//! for the H32 budgets: only a speculation exit (a fact the code was compiled from did not
//! hold) counts toward them; an environment exit is a lazy deopt from an outside event; a
//! not-filled exit is taken because the feedback its guard reads is not there yet (no
//! feedback vector, or an IC entry T0 has not filled), and it is outside the site and
//! function budgets. No code reads a budget yet. The reason says which check of the guard
//! failed (one record per cause is the writer's choice, not a rule); 0 = unknown, for a
//! guard with one check. The reader checks it; no VM code acts on it yet: it is kept for a
//! later recompile, and for AOT code it is in the unit's rodata (F14, H35).
//!
//! The reader refuses every unknown value (H29 Freeze table: "the deopt reader refuses a code
//! object with an unknown tag"); the install then refuses the code, and the function stays in
//! T0. The writer is the compiler's (Spike C's `deopt_meta.rs` today; `ep9_opt` later, H29
//! Crates); `Table::encode` is the tests' reference writer. DEOPT_META_VERSION stays 0 in
//! month 6: no shipped metadata (V10-D29); the form above is "format 1 (declared)", and the
//! version moves to 1 at A2 start (V11-D12).
//!
//! **No GC point** (H29-Q7, SCP:127). The rebuild reads every value before it writes one,
//! boxes ints, booleans and doubles as immediates and allocates nothing on the heap: a
//! materialize location (an object removed by escape analysis) is not built.

use core::cell::RefCell;
use std::collections::BTreeMap;

use ep9_bytecode::Register;
use ep9_value::Value;

use crate::frame::slot;

// ---- the H29 byte form ----

/// The tag values and bit fields of the H29 byte form (module docs).
pub(crate) mod form {
    /// Record header bits 0-3: the record tag.
    pub const RECORD_TAG_MASK: u8 = 0x0F;
    /// Record tag 0: an eager exit (a guard).
    pub const RECORD_EXIT: u8 = 0;
    /// Record tag 1: an interrupt poll (a lazy-deopt point).
    pub const RECORD_POLL: u8 = 1;
    /// Record tag 2: a call return point (a lazy deopt lands there).
    pub const RECORD_CALL: u8 = 2;
    /// The record tags this reader knows: `0..RECORD_TAGS` (append-only, H29).
    pub const RECORD_TAGS: u8 = 3;
    /// Record header bits 4-7: the exit class (H29-Q1: a 4-bit append-only list).
    pub const CLASS_SHIFT: u32 = 4;
    /// Exit class 0: a speculation exit (a fact the code was compiled from did not hold).
    pub const CLASS_SPECULATION: u8 = 0;
    /// Exit class 1: an environment exit (a lazy deopt from an outside event).
    pub const CLASS_ENVIRONMENT: u8 = 1;
    /// Exit class 2: a not-filled exit (no feedback vector, or an unfilled IC entry; V11-D14).
    pub const CLASS_NOT_FILLED: u8 = 2;
    /// The exit classes this reader knows: `0..CLASSES` (append-only, H29).
    pub const CLASSES: u8 = 3;

    /// Record field `reason` 0: unknown (a guard with one check, or a writer that names none).
    pub const REASON_UNKNOWN: u32 = 0;
    /// Reason 1: the IC entry holds a case, but not the guard's (a shape guard).
    pub const REASON_SHAPE_DIFFERS: u32 = 1;
    /// Reason 2: the value is not a cell (a shape guard).
    pub const REASON_NOT_CELL: u32 = 2;
    /// Reason 3: the function has no feedback vector.
    pub const REASON_NO_VECTOR: u32 = 3;
    /// Reason 4: the IC entry is unfilled (its key half is undefined).
    pub const REASON_UNFILLED: u32 = 4;
    /// Reason 5: the IC entry is megamorphic.
    pub const REASON_MEGAMORPHIC: u32 = 5;
    /// The reasons this reader knows: `0..REASONS` (append-only, V11-D13).
    pub const REASONS: u32 = 6;

    /// Value tag bits 0-3: the location tag.
    pub const LOC_MASK: u8 = 0x0F;
    /// Location 0: a machine register of the exit's register dump.
    pub const LOC_REG: u8 = 0;
    /// Location 1: a native stack slot, by its byte offset from the exit's frame pointer.
    pub const LOC_STACK: u8 = 1;
    /// Location 2: an entry of the table's const list.
    pub const LOC_CONST: u8 = 2;
    /// Location 3: a VM frame slot, by its byte offset from fp (the M5-P1 fill).
    pub const LOC_FRAME: u8 = 3;
    /// The location tags this reader knows: `0..LOC_TAGS` (append-only, H29).
    pub const LOC_TAGS: u8 = 4;
    /// Value tag bits 4-6: the repr.
    pub const REPR_SHIFT: u32 = 4;
    pub const REPR_MASK: u8 = 0x70;
    pub const REPR_TAGGED: u8 = 0;
    pub const REPR_INT32: u8 = 1;
    pub const REPR_BOOL: u8 = 2;
    pub const REPR_FLOAT64: u8 = 3;
    /// The reprs this reader knows: `0..REPRS`.
    pub const REPRS: u8 = 4;
    /// Value tag bit 7: the target is the accumulator.
    pub const ACC: u8 = 0x80;
}

// Each list's length is its last value + 1: the tests fill each list, then probe the top of
// its field (tag 15, class 15, repr 7, reason u32::MAX), never the next free value: a later
// append fills that one (w17 O4; README append rule, v11in #6).
const _: () = assert!(
    form::RECORD_TAGS == form::RECORD_CALL + 1
        && form::CLASSES == form::CLASS_NOT_FILLED + 1
        && form::REASONS == form::REASON_MEGAMORPHIC + 1
        && form::LOC_TAGS == form::LOC_FRAME + 1
        && form::REPRS == form::REPR_FLOAT64 + 1
);

/// The registers of an exit's register dump: x0-x30 (AArch64, H29-Q3), one word each, at
/// index = register number.
pub(crate) const DUMP_REGS: usize = 31;

/// What a record is for (its record tag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordTag {
    Exit,
    Poll,
    Call,
}

/// What an exit means for the H32 budgets (H29-Q1, C-04; the module docs): only speculation
/// exits count toward them; an environment exit is a lazy deopt from an outside event; a
/// not-filled exit (V11-D14) is outside the site and function budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitClass {
    Speculation,
    Environment,
    NotFilled,
}

/// Which check of a guard failed (V11-D13; the record's `reason` field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitReason {
    /// A guard with one check, or a writer that names no reason.
    Unknown,
    /// The IC entry holds a case, but not the guard's: the fact did not hold.
    ShapeDiffers,
    /// The value is not a cell.
    NotCell,
    /// The function has no feedback vector.
    NoVector,
    /// The IC entry is unfilled.
    Unfilled,
    /// The IC entry is megamorphic.
    Megamorphic,
}

/// The machine form of a value at an exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Repr {
    /// A Value.
    Tagged,
    /// An int32 in the low 32 bits.
    Int32,
    /// 0 or 1 in the low 32 bits.
    Bool,
    /// The IEEE bits of a double.
    Float64,
}

/// Where a value is at an exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Loc {
    /// Machine register `x<n>` (index into the register dump).
    Reg(u8),
    /// The native stack word at this byte offset from the exit's frame pointer.
    Stack(u32),
    /// Entry `i` of the table's const list.
    Const(u32),
    /// The VM frame slot at this byte offset from fp.
    Frame(i32),
}

/// Where a value goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Acc,
    Reg(Register),
}

/// One value of a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExitValue {
    pub target: Target,
    pub repr: Repr,
    pub loc: Loc,
}

/// One record: an exit stub's pc and what T0 needs to resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub tag: RecordTag,
    pub class: ExitClass,
    pub pc: u32,
    pub bytecode: u32,
    pub counter: u32,
    pub feedback: Option<u32>,
    pub reason: ExitReason,
    pub values: Vec<ExitValue>,
}

/// The metadata of one code object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Table {
    /// Immediate Values (never a cell: H30 has no raw heap constant, plan rule 8).
    pub consts: Vec<u64>,
    /// Sorted by pc.
    pub records: Vec<Record>,
}

/// A reader over the table bytes.
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
            if shift >= 64 || (shift == 63 && b > 1) {
                return Err("a uleb is too long".into());
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

    /// A signed field: ZigZag + uleb (module docs), as an i32.
    fn zigzag32(&mut self) -> Result<i32, String> {
        let z = self.uleb()?;
        let v = ((z >> 1) as i64) ^ -((z & 1) as i64);
        i32::try_from(v).map_err(|_| "an i32 field is too large".to_string())
    }
}

impl Table {
    /// Reads a table in the H29 byte form. Refuses every unknown tag, class, reason or repr
    /// value, a cut or overlong table, a const that is a cell, records out of pc order, a
    /// counter index past the records, an inline frame, a register past the dump, a stack
    /// or frame offset that is not a word, a const index past the list and two values for
    /// one target.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Table, String> {
        let mut r = Reader { b: bytes, at: 0 };
        let n = r.u32()?;
        let nc = r.u32()?;
        let mut t = Table::default();
        for i in 0..nc {
            let s = bytes.get(r.at..r.at + 8).ok_or("truncated const")?;
            let bits = u64::from_le_bytes(s.try_into().expect("8 bytes"));
            if !Value::is_valid_bits(bits) || is_cell_bits(bits) {
                return Err(format!("const {i} ({bits:#x}) is not an immediate Value"));
            }
            t.consts.push(bits);
            r.at += 8;
        }
        for k in 0..n {
            let rec = decode_record(&mut r, &t.consts, n)
                .map_err(|e| format!("record {k} (byte {}): {e}", r.at))?;
            if let Some(prev) = t.records.last()
                && prev.pc >= rec.pc
            {
                return Err(format!(
                    "record {k}: pc {} after pc {} (records are sorted by pc)",
                    rec.pc, prev.pc
                ));
            }
            t.records.push(rec);
        }
        if r.at != bytes.len() {
            return Err(format!(
                "{} bytes after the last record",
                bytes.len() - r.at
            ));
        }
        Ok(t)
    }

    /// The record at native offset `pc`.
    pub(crate) fn record_at(&self, pc: u32) -> Option<&Record> {
        self.records
            .binary_search_by_key(&pc, |r| r.pc)
            .ok()
            .map(|i| &self.records[i])
    }

    /// The tests' reference writer of the H29 byte form (the compilers write their own).
    #[cfg(test)]
    pub(crate) fn encode(&self) -> Vec<u8> {
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
        fn zigzag(out: &mut Vec<u8>, v: i64) {
            uleb(out, ((v << 1) ^ (v >> 63)) as u64);
        }
        let mut out = Vec::new();
        uleb(&mut out, self.records.len() as u64);
        uleb(&mut out, self.consts.len() as u64);
        for &c in &self.consts {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for r in &self.records {
            let tag = match r.tag {
                RecordTag::Exit => form::RECORD_EXIT,
                RecordTag::Poll => form::RECORD_POLL,
                RecordTag::Call => form::RECORD_CALL,
            };
            let class = match r.class {
                ExitClass::Speculation => form::CLASS_SPECULATION,
                ExitClass::Environment => form::CLASS_ENVIRONMENT,
                ExitClass::NotFilled => form::CLASS_NOT_FILLED,
            };
            let reason = match r.reason {
                ExitReason::Unknown => form::REASON_UNKNOWN,
                ExitReason::ShapeDiffers => form::REASON_SHAPE_DIFFERS,
                ExitReason::NotCell => form::REASON_NOT_CELL,
                ExitReason::NoVector => form::REASON_NO_VECTOR,
                ExitReason::Unfilled => form::REASON_UNFILLED,
                ExitReason::Megamorphic => form::REASON_MEGAMORPHIC,
            };
            out.push(tag | class << form::CLASS_SHIFT);
            for f in [
                r.pc,
                r.bytecode,
                r.counter,
                r.feedback.map_or(0, |s| s + 1),
                reason,
                0,
            ] {
                uleb(&mut out, u64::from(f));
            }
            uleb(&mut out, r.values.len() as u64);
            for v in &r.values {
                let (loc, payload) = match v.loc {
                    Loc::Reg(n) => (form::LOC_REG, i64::from(n)),
                    Loc::Stack(o) => (form::LOC_STACK, i64::from(o)),
                    Loc::Const(i) => (form::LOC_CONST, i64::from(i)),
                    Loc::Frame(o) => (form::LOC_FRAME, i64::from(o)),
                };
                let repr = match v.repr {
                    Repr::Tagged => form::REPR_TAGGED,
                    Repr::Int32 => form::REPR_INT32,
                    Repr::Bool => form::REPR_BOOL,
                    Repr::Float64 => form::REPR_FLOAT64,
                };
                let acc = if v.target == Target::Acc {
                    form::ACC
                } else {
                    0
                };
                out.push(loc | repr << form::REPR_SHIFT | acc);
                if let Target::Reg(t) = v.target {
                    zigzag(&mut out, i64::from(t.to_operand()));
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
}

/// True for the bits of a cell (no number tag, no immediate tag bit).
fn is_cell_bits(bits: u64) -> bool {
    bits & ep9_value::encoding::NOT_CELL_MASK == 0
}

/// One record of a table with `records` records and the const list `consts`.
fn decode_record(r: &mut Reader<'_>, consts: &[u64], records: u32) -> Result<Record, String> {
    let header = r.byte()?;
    let tag = match header & form::RECORD_TAG_MASK {
        form::RECORD_EXIT => RecordTag::Exit,
        form::RECORD_POLL => RecordTag::Poll,
        form::RECORD_CALL => RecordTag::Call,
        t => return Err(format!("unknown record tag {t}")),
    };
    let class = match header >> form::CLASS_SHIFT {
        form::CLASS_SPECULATION => ExitClass::Speculation,
        form::CLASS_ENVIRONMENT => ExitClass::Environment,
        form::CLASS_NOT_FILLED => ExitClass::NotFilled,
        c => return Err(format!("unknown exit class {c}")),
    };
    let (pc, bytecode, counter) = (r.u32()?, r.u32()?, r.u32()?);
    if counter >= records {
        return Err(format!("counter {counter} of {records} records"));
    }
    let feedback = r.u32()?.checked_sub(1);
    let reason = match r.u32()? {
        form::REASON_UNKNOWN => ExitReason::Unknown,
        form::REASON_SHAPE_DIFFERS => ExitReason::ShapeDiffers,
        form::REASON_NOT_CELL => ExitReason::NotCell,
        form::REASON_NO_VECTOR => ExitReason::NoVector,
        form::REASON_UNFILLED => ExitReason::Unfilled,
        form::REASON_MEGAMORPHIC => ExitReason::Megamorphic,
        x => return Err(format!("unknown exit reason {x}")),
    };
    let inline = r.uleb()?;
    if inline != 0 {
        return Err(format!(
            "{inline} inline frames (no inline frame kind is built)"
        ));
    }
    let nv = r.u32()?;
    let mut values: Vec<ExitValue> = Vec::new();
    for i in 0..nv {
        let v = decode_value(r, consts).map_err(|e| format!("value {i}: {e}"))?;
        if values.iter().any(|w| w.target == v.target) {
            return Err(format!("value {i}: a second value for {:?}", v.target));
        }
        values.push(v);
    }
    Ok(Record {
        tag,
        class,
        pc,
        bytecode,
        counter,
        feedback,
        reason,
        values,
    })
}

fn decode_value(r: &mut Reader<'_>, consts: &[u64]) -> Result<ExitValue, String> {
    let tag = r.byte()?;
    let repr = match (tag & form::REPR_MASK) >> form::REPR_SHIFT {
        form::REPR_TAGGED => Repr::Tagged,
        form::REPR_INT32 => Repr::Int32,
        form::REPR_BOOL => Repr::Bool,
        form::REPR_FLOAT64 => Repr::Float64,
        x => return Err(format!("unknown repr {x}")),
    };
    let target = if tag & form::ACC != 0 {
        Target::Acc
    } else {
        Target::Reg(Register::from_operand(r.zigzag32()?))
    };
    let word = |o: i64| -> Result<(), String> {
        if o % 8 == 0 {
            Ok(())
        } else {
            Err(format!("byte offset {o} is not a word"))
        }
    };
    let loc = match tag & form::LOC_MASK {
        form::LOC_REG => {
            let n = r.u32()?;
            if n as usize >= DUMP_REGS {
                return Err(format!("register x{n} is not in the dump"));
            }
            Loc::Reg(n as u8)
        }
        form::LOC_STACK => {
            let o = r.u32()?;
            word(i64::from(o))?;
            Loc::Stack(o)
        }
        form::LOC_CONST => {
            let i = r.u32()?;
            if i as usize >= consts.len() {
                return Err(format!("const {i} of {}", consts.len()));
            }
            Loc::Const(i)
        }
        form::LOC_FRAME => {
            let o = r.zigzag32()?;
            word(i64::from(o))?;
            Loc::Frame(o)
        }
        x => return Err(format!("unknown location tag {x}")),
    };
    Ok(ExitValue { target, repr, loc })
}

// ---- the frame rebuild ----

/// The machine state of an exit, as the shared exit saved it.
pub(crate) struct ExitState<'a> {
    /// x0-x30 at the exit (index = register number).
    pub regs: &'a [u64; DUMP_REGS],
    /// The exit's native frame pointer: a stack value is the word at this address plus its
    /// offset.
    pub native_fp: *const u8,
}

/// The slots of a VM frame that a deopt may write or read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameShape {
    /// `r0 .. r<registers - 1>`.
    pub registers: u32,
    /// `a0 .. a<params - 1>`: max(argc, parameter count).
    pub params: u32,
}

impl FrameShape {
    /// The slot (word offset from fp) of register `r`, if the frame has it.
    fn slot_of(self, r: Register) -> Option<isize> {
        if r.is_receiver() {
            Some(slot::RECEIVER)
        } else if let Some(i) = r.local_index() {
            (i < self.registers).then_some(i as isize)
        } else {
            let k = r.parameter_index()?;
            (k < self.params).then_some(slot::PARAM0 - k as isize)
        }
    }

    /// The slot at byte offset `off` from fp, if it is the receiver, a parameter or a
    /// register of the frame (never a header word).
    fn slot_at(self, off: i32) -> Option<isize> {
        let s = off as isize / 8;
        let receiver = s == slot::RECEIVER;
        let param = s <= slot::PARAM0 && slot::PARAM0 - s < self.params as isize;
        let local = (0..self.registers as isize).contains(&s);
        (receiver || param || local).then_some(s)
    }
}

/// Where T0 goes on after a deopt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resume {
    /// The bytecode offset to run next.
    pub bytecode: u32,
    /// The accumulator there (undefined if the record has no accumulator value: it is dead).
    pub acc: u64,
}

/// The Value of the machine word `bits` of form `repr` (no allocation: an int32, a boolean
/// and a double box as immediates, H29-Q7). `None` for Tagged bits no Value has.
fn box_value(repr: Repr, bits: u64) -> Option<u64> {
    let v = match repr {
        Repr::Tagged => return Value::is_valid_bits(bits).then_some(bits),
        Repr::Int32 => Value::int32(bits as u32 as i32),
        Repr::Bool => Value::bool(bits as u32 != 0),
        Repr::Float64 => Value::number(f64::from_bits(bits)),
    };
    Some(v.to_bits())
}

/// Rebuilds the T0 state of the record `rec` of `table` in the VM frame `fp` (of `shape`)
/// from the machine state `exit`: reads every value first, boxes it by its repr, then writes
/// the VM registers. Returns where T0 resumes and its accumulator. Changes nothing when it
/// refuses (a poll record, a target or frame slot the frame does not have,
/// Tagged bits no Value has).
///
/// # Safety
/// `fp` is a live VM frame with the slots `shape` names; for each stack value of `rec`,
/// `exit.native_fp` plus its offset is a readable word of the exit's native frame.
pub(crate) unsafe fn rebuild(
    table: &Table,
    rec: &Record,
    exit: &ExitState<'_>,
    fp: *mut u64,
    shape: FrameShape,
) -> Result<Resume, String> {
    // An exit, or a call record (a lazy exit, SPIKEC-10): both resume T0 at the record's
    // offset. A poll record is not built.
    if rec.tag == RecordTag::Poll {
        return Err(format!(
            "the record at pc {} is a {:?} record, not an exit or a call",
            rec.pc, rec.tag
        ));
    }
    let mut writes: Vec<(Option<isize>, u64)> = Vec::with_capacity(rec.values.len());
    for v in &rec.values {
        let bits = match v.loc {
            Loc::Reg(n) => exit.regs[n as usize],
            // SAFETY: the caller's contract (a word of the exit's native frame).
            Loc::Stack(o) => unsafe { exit.native_fp.add(o as usize).cast::<u64>().read() },
            Loc::Const(i) => table.consts[i as usize],
            Loc::Frame(o) => {
                let s = shape
                    .slot_at(o)
                    .ok_or_else(|| format!("frame offset {o} is not a value slot of the frame"))?;
                // SAFETY: `s` is a slot of the live frame `fp` (caller contract, `shape`).
                unsafe { *fp.offset(s) }
            }
        };
        let boxed = box_value(v.repr, bits)
            .ok_or_else(|| format!("{:?} holds {bits:#x}, not a Value", v.loc))?;
        let slot = match v.target {
            Target::Acc => None,
            Target::Reg(r) => Some(
                shape
                    .slot_of(r)
                    .ok_or_else(|| format!("the frame has no register {r}"))?,
            ),
        };
        writes.push((slot, boxed));
    }
    let mut acc = Value::UNDEFINED.to_bits();
    for (slot, bits) in writes {
        match slot {
            // SAFETY: `s` is a slot of the live frame `fp` (`FrameShape::slot_of`).
            Some(s) => unsafe { *fp.offset(s) = bits },
            None => acc = bits,
        }
    }
    Ok(Resume {
        bytecode: rec.bytecode,
        acc,
    })
}

// ---- the side table ----

/// The deopt metadata of one code object, read and checked ([`CodeMeta::read`]): what
/// [`SideTable::insert`] keeps for it.
pub(crate) struct CodeMeta {
    /// Its function's FunctionInfo address.
    function: usize,
    /// Its length in bytes: an exit's pc offset is below it.
    len: usize,
    table: Table,
    /// The RW exit counters, one per record (H29-Q5).
    counters: Vec<u64>,
}

impl CodeMeta {
    /// Reads `bytes` (the H29 form) as the metadata of `len` code bytes that run the function
    /// whose FunctionInfo is at `function`, with zeroed counters. Refuses a table the reader
    /// refuses, a record whose pc lies outside the code, and a record that resumes T0 at an
    /// offset that is not an instruction start of the function's bytecode (`insn_start`).
    pub(crate) fn read(
        len: usize,
        function: usize,
        bytes: &[u8],
        insn_start: impl Fn(u32) -> bool,
    ) -> Result<CodeMeta, String> {
        let table = Table::decode(bytes)?;
        for r in &table.records {
            if r.pc as usize >= len {
                return Err(format!("record at pc {} of {len} code bytes", r.pc));
            }
            if !insn_start(r.bytecode) {
                return Err(format!(
                    "record at pc {} resumes at bytecode offset {}, not an instruction start",
                    r.pc, r.bytecode
                ));
            }
        }
        let counters = vec![0; table.records.len()];
        Ok(CodeMeta {
            function,
            len,
            table,
            counters,
        })
    }
}

/// The per-isolate deopt side table (V10-D32; `Runtime::deopt`): code start -> the code's
/// decoded deopt metadata and its exit counters, filled at install. Rust memory that holds no
/// Value across a GC point (plan rule 8: the consts are immediates). Not in `CodeRegistry`:
/// a `dispatch.rs` change would need an SP-Dispatch row (rule 22).
pub(crate) struct SideTable {
    code: RefCell<BTreeMap<usize, CodeMeta>>,
}

impl SideTable {
    /// The empty table of a new isolate.
    pub(crate) fn new() -> SideTable {
        SideTable {
            code: RefCell::new(BTreeMap::new()),
        }
    }

    /// True if no code has metadata here.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.code.borrow().is_empty()
    }

    /// Keeps `meta` for the code at `start`. Replaces what an earlier code at `start` left.
    pub(crate) fn insert(&self, start: usize, meta: CodeMeta) {
        self.code.borrow_mut().insert(start, meta);
    }

    /// Drops what the code at `start` left: its metadata and counters
    /// (`native::uninstall_native`, built with the `spike-native` feature, calls it when the code leaves its function, so a
    /// later code at the same start never reads them). True if there was an entry.
    pub(crate) fn remove(&self, start: usize) -> bool {
        self.code.borrow_mut().remove(&start).is_some()
    }

    /// The exit counters of the code at `start`, in record order.
    pub(crate) fn counters(&self, start: usize) -> Option<Vec<u64>> {
        self.code.borrow().get(&start).map(|c| c.counters.clone())
    }

    /// The eager deopt at the exit stub at offset `pc` of the code at `start`, which runs the
    /// function whose FunctionInfo is at `function` in the VM frame `fp` (of `shape`):
    /// [`rebuild`], then the exit's counter + 1. Refuses code with no metadata here, code of
    /// another function, and a pc with no record; then nothing changes.
    ///
    /// # Safety
    /// As [`rebuild`].
    pub(crate) unsafe fn deopt(
        &self,
        start: usize,
        function: usize,
        pc: u32,
        exit: &ExitState<'_>,
        fp: *mut u64,
        shape: FrameShape,
    ) -> Result<Resume, String> {
        let mut code = self.code.borrow_mut();
        let meta = code
            .get_mut(&start)
            .ok_or_else(|| format!("no deopt metadata for the code at {start:#x}"))?;
        if meta.function != function {
            return Err("the code's metadata belongs to another function".into());
        }
        if pc as usize >= meta.len {
            return Err(format!("exit pc {pc} of {} code bytes", meta.len));
        }
        let rec = meta
            .table
            .record_at(pc)
            .ok_or_else(|| format!("no exit record at pc {pc}"))?;
        // SAFETY: the caller's contract.
        let resume = unsafe { rebuild(&meta.table, rec, exit, fp, shape) }?;
        let c = &mut meta.counters[rec.counter as usize];
        *c = c.saturating_add(1);
        Ok(resume)
    }
}

// ---- the lazy swap (SPIKEC-10, H29-Q4; the short probe) ----
//
// **Lazy deopt, the short probe** (M6-P2 SPIKEC-10; H29-Q4; plan v11 §4.8 task 1, the V11-D28
// speed branch). A call record (tag 2) is a call site's lazy exit: a swap ([`swap_returns`])
// writes the code's lazy exit of that site over the call's return address in a frame
// record of the native stack ([`walk_records`]: the AArch64 frame-record chain, AAPCS64
// §6.2.3), so the call returns into the lazy exit, which takes the shared exit with the
// record's pc as an exit stub does. [`rebuild`] then resumes T0 at the record's offset (the
// instruction after the call) with the call's result as the accumulator. The walk and the
// swap are here, with no cfg (C-10); reading the live x29 is the `spike-native` glue
// (`native.rs`). What the probe does not prove (a CFI walk where frame records are not
// kept, signed return addresses, a RuntimeTable entry's own return, freeing dead code) is
// the gated part of H29 (H29-Q4).
//
// ```text
//  compiled f --blr--> call_any (Rust frame record: [x29] = f's x29, [x29+8] = ret into f)
//     --> callee runs; an event marks f's code dead --> walk_records from the current x29
//     --> the record whose ret = f's code + the site's return offset: ret := code + lazy exit
//  callee returns --> call_any returns into f's lazy exit (x0 = the result)
//     --> restore what the call saved --> shared exit --> rebuild(call record)
//     --> T0 resumes after the call, acc = x0
// ```
//

/// One AArch64 frame record (AAPCS64 §6.2.3): two words at `at`, the caller's x29 and the
/// return address `ret`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameRecord {
    pub at: usize,
    pub ret: usize,
}

/// The frame records of the chain that starts at `fp` (a live x29), at most `max`: it stops
/// at a null or misaligned record address, at one that is not above the record before it
/// (the stack grows down, so each caller's record is higher), and at one at or past `end`.
///
/// # Safety
/// Every record address the walk reaches below `end` is a readable 16-byte record of a live
/// stack (on macOS arm64 every frame keeps one: the platform ABI makes x29 a frame-record
/// chain that ends at 0).
pub(crate) unsafe fn walk_records(fp: usize, end: usize, max: usize) -> Vec<FrameRecord> {
    let mut out = Vec::new();
    let mut at = fp;
    while out.len() < max && at != 0 && at.is_multiple_of(16) && at < end {
        let p = core::ptr::with_exposed_provenance::<usize>(at);
        // SAFETY: the caller's contract (a readable record below `end`).
        let (next, ret) = unsafe { (p.read(), p.add(1).read()) };
        out.push(FrameRecord { at, ret });
        if next <= at {
            break;
        }
        at = next;
    }
    out
}

/// The lazy swap of the code at `code` (`len` bytes) with the call sites `sites` (the native
/// offsets of each site's return address and lazy exit): every record of `records` whose
/// return address is `code + ret` of a site gets `code + stub` instead. Returns each swapped
/// record with its old return address. A return address that is no site's is left alone
/// (a frame of other code, or a site already swapped), so a second swap changes nothing.
///
/// # Safety
/// Each record of `records` is a live frame record (from [`walk_records`] on the current
/// stack, with no frame left since), and each site's lazy exit is code that runs the call's
/// return as the module docs say.
pub(crate) unsafe fn swap_returns(
    records: &[FrameRecord],
    code: usize,
    len: usize,
    sites: &[(u32, u32)],
) -> Vec<(FrameRecord, usize)> {
    let mut out = Vec::new();
    for r in records {
        let Some(off) = r.ret.checked_sub(code).filter(|&o| o < len) else {
            continue;
        };
        let Some(&(_, stub)) = sites.iter().find(|&&(ret, _)| ret as usize == off) else {
            continue;
        };
        let p = core::ptr::with_exposed_provenance_mut::<usize>(r.at);
        // SAFETY: the caller's contract (a live record: its second word is the return address).
        unsafe { p.add(1).write(code + stub as usize) };
        out.push((*r, r.ret));
    }
    out
}
