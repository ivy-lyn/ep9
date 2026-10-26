//! The VM layout facts the emitted code reads that `ep9_vm::layout` does not export yet:
//! object, array and ELEMENTS offsets, kind ids, the named-load IC (feedback slot, smi
//! handler, POLY_IC cell) and the offset of `FunctionInfo.feedback`. They are copied from the
//! ep9_vm module docs at the M4-P0 head (OBJECT_LAYOUT 4, IC_LAYOUT 2), and [`check`] proves
//! them on live objects before any compile ([`crate::compile`] runs it once per session).
//!
//! | Fact | Value | ep9_vm source (private) |
//! |---|---|---|
//! | word0: ShapeId in bytes 0-3, kind in byte 5 | `ldr w` at +0, `ldrb` at +5 | `ic.rs` "hit: (key as u32) == word0.shape_id"; `ep9_heap` cell.rs |
//! | OBJECT kind 16, ARRAY kind 18 | | `kinds.rs` |
//! | in-object field `f` of an OBJECT (no fixed fields) | +24 + 8f | `object.rs` `obj::FIXED`, `ic.rs` `field_location` |
//! | ARRAY: ELEMENTS at +16, length (int32 Value) at +24 | | `object.rs` `obj::ELEMENTS`, `arr::LENGTH` |
//! | ELEMENTS: capacity (int32 Value) at +8, values from +16, a hole is `Value::EMPTY` | | `object.rs` `elems` |
//! | FEEDBACK cell: slot `i` at +24 + 16i: key (weak), then value | | `feedback.rs` `fv` |
//! | smi handler: int32, kind in bits 0-3 (FIELD_IN_OBJECT = 1, ARRAY_LENGTH = 3), byte offset in bits 4-31 | | `ic.rs` `smi` |
//! | POLY_IC: case `i` at +8 + 16i: key (weak), then handler | | `ic.rs` `pc` |
//! | `FunctionInfo.feedback`: a pointer to the unit slot that holds the FEEDBACK cell (null: none) | [`FunctionInfoMirror`] | `function_info.rs` (`repr(C)`) |
//!
//! These are a plan v9 rule 12 need: `crates/ep9_vm/src/native.rs` has no month-4 owner, so
//! the spike keeps the copies and the checks, and the integration fixer (or month 5) moves
//! them into `ep9_vm::layout`.

use std::cell::Cell;
use std::mem::offset_of;

use ep9_value::encoding::{DOUBLE_ENCODE_OFFSET, NOT_CELL_MASK, NUMBER_TAG};
use ep9_vm::{Value, layout};

/// The kind byte of a cell (word0 byte 5).
pub const KIND_BYTE: u32 = 5;
/// Kind id of an ordinary object.
pub const KIND_OBJECT: u8 = 16;
/// Kind id of an Array.
pub const KIND_ARRAY: u8 = 18;
/// First in-object field of an OBJECT (it has no fixed fields).
pub const OBJECT_FIELDS: u32 = 24;
/// ARRAY: the ELEMENTS cell (or undefined).
pub const ARRAY_ELEMENTS: u32 = 16;
/// ARRAY: the length (an int32 Value).
pub const ARRAY_LENGTH: u32 = 24;
/// ELEMENTS: the capacity (an int32 Value).
pub const ELEMENTS_CAPACITY: u32 = 8;
/// ELEMENTS: the first value.
pub const ELEMENTS_DATA: u32 = 16;
/// The hole (`Value::EMPTY`).
pub const HOLE: u64 = Value::EMPTY.to_bits();
/// FEEDBACK cell: the first slot.
pub const FEEDBACK_SLOTS: u32 = 24;
/// FEEDBACK cell: bytes per slot (key, then value).
pub const FEEDBACK_SLOT_SIZE: u32 = 16;
/// POLY_IC: the first case (key, then handler).
pub const POLY_CASES: u32 = 8;
/// POLY_IC: bytes per case.
pub const POLY_CASE_SIZE: u32 = 16;
/// POLY_IC: cases per cell.
pub const POLY_COUNT: u32 = 4;
/// Smi handler kind: the field at the byte offset of the object cell.
pub const SMI_FIELD_IN_OBJECT: i32 = 1;
/// Smi handler kind: an Array's `length` (the int32 at [`ARRAY_LENGTH`]).
pub const SMI_ARRAY_LENGTH: i32 = 3;
/// The smi handler of `length` on an Array shape.
pub const ARRAY_LENGTH_HANDLER: u64 = NUMBER_TAG | SMI_ARRAY_LENGTH as u32 as u64;
/// Smi handler: bits 0-3 are the kind.
pub const SMI_KIND_MASK: i32 = 0xF;
/// Smi handler: the byte offset starts at bit 4.
pub const SMI_SHIFT: u32 = 4;

/// The smi handler (an int32 Value) of an in-object field at byte offset `offset`.
pub const fn field_in_object_handler(offset: u32) -> u64 {
    NUMBER_TAG | (SMI_FIELD_IN_OBJECT | ((offset as i32) << SMI_SHIFT)) as u32 as u64
}

/// `repr(C)` copy of `ep9_vm`'s `FunctionInfo` (function_info.rs; "New fields go at the end"),
/// only to find the byte offset of its `feedback` field. Every field before it has the same
/// type or the same size and alignment as the original; [`check`] compares the offsets that
/// `ep9_vm::layout` does export (code, counts, dispatch slot) and then follows the `feedback`
/// pointer of a live function back to the same FunctionInfo.
#[repr(C)]
pub struct FunctionInfoMirror {
    slot: ep9_vm::DispatchSlot,
    register_count: u32,
    parameter_count: u32,
    flags: u32,
    function_id: u32,
    code: *const u8,
    code_len: u32,
    length: u32,
    consts: *const u64,
    unit: *const u8,
    host: Option<fn()>,
    name: String,
    kind: Option<ep9_bytecode::FunctionKind>,
    feedback: Cell<*const u64>,
    invocations: Cell<u32>,
}

/// Byte offset of `FunctionInfo.feedback`.
pub const FI_FEEDBACK: usize = offset_of!(FunctionInfoMirror, feedback);

/// The offsets `ep9_vm::layout` exports agree with the mirror.
pub fn mirror_agrees() -> Result<(), String> {
    let pairs = [
        (
            "code",
            offset_of!(FunctionInfoMirror, code),
            layout::FI_CODE,
        ),
        (
            "register_count",
            offset_of!(FunctionInfoMirror, register_count),
            layout::FI_REGISTER_COUNT,
        ),
        (
            "parameter_count",
            offset_of!(FunctionInfoMirror, parameter_count),
            layout::FI_PARAMETER_COUNT,
        ),
        (
            "slot",
            offset_of!(FunctionInfoMirror, slot) + ep9_vm::DispatchSlot::KIND_OFFSET,
            layout::FI_SLOT_KIND,
        ),
    ];
    for (name, mine, vm) in pairs {
        if mine != vm {
            return Err(format!(
                "FunctionInfo.{name}: the mirror says {mine}, ep9_vm::layout {vm}"
            ));
        }
    }
    Ok(())
}

/// True if `bits` is a cell (no number tag, no immediate tag bit).
pub const fn is_cell(bits: u64) -> bool {
    bits & NOT_CELL_MASK == 0 && bits != 0
}

/// The word at `addr + off`.
///
/// # Safety
/// `addr + off` is a readable, 8-aligned word (a field of a live cell or FunctionInfo).
unsafe fn word(addr: u64, off: u32) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { ((addr + u64::from(off)) as *const u64).read() }
}

/// The FEEDBACK cell of the function whose FunctionInfo is at `info`, through
/// [`FI_FEEDBACK`], checked by the cell's back pointer (+16: the FunctionInfo as a native
/// word). `None` if the function has no vector yet.
///
/// # Safety
/// `info` is the address of a live FunctionInfo (`NativeTarget::info`), and its isolate runs
/// nothing while this reads.
pub unsafe fn feedback_cell(info: usize) -> Result<Option<u64>, String> {
    mirror_agrees()?;
    // SAFETY: `info` is a live FunctionInfo (caller); FI_FEEDBACK is a field inside it (the
    // mirror agrees with ep9_vm::layout on every field it can compare).
    let slot = unsafe { word(info as u64, FI_FEEDBACK as u32) };
    if slot == 0 {
        return Ok(None);
    }
    if !slot.is_multiple_of(8) {
        return Err(format!(
            "FunctionInfo.feedback {slot:#x} is not a slot pointer"
        ));
    }
    // SAFETY: a non-null `feedback` points at the function's slot in its unit's feedback
    // root range (function_info.rs), a Value.
    let cell = unsafe { word(slot, 0) };
    if !is_cell(cell) {
        return Err(format!("feedback slot holds {cell:#x}, not a cell"));
    }
    // SAFETY: `cell` is the live FEEDBACK cell; +16 is its FunctionInfo word.
    let back = unsafe { word(cell, 16) };
    if back != info as u64 + layout::NATIVE_WORD_OFFSET {
        return Err(format!(
            "FEEDBACK cell +16 is {back:#x}, not the FunctionInfo {info:#x} (layout drift)"
        ));
    }
    Ok(Some(cell))
}

/// Slot `i` (key, value) of the FEEDBACK cell `cell`.
///
/// # Safety
/// `cell` is a live FEEDBACK cell with more than `i` slots, and its isolate runs nothing.
pub unsafe fn feedback_slot(cell: u64, i: u32) -> (u64, u64) {
    let at = FEEDBACK_SLOTS + FEEDBACK_SLOT_SIZE * i;
    // SAFETY: the caller's contract.
    unsafe { (word(cell, at), word(cell, at + 8)) }
}

/// The handlers the named-load IC entry `(key, value)` holds for objects whose ShapeId is
/// `id`, as T0's hit path finds them (ic.rs `handler_for`).
///
/// # Safety
/// `(key, value)` is a live IC entry of a FEEDBACK cell, and its isolate runs nothing.
pub unsafe fn ic_handler_for(key: u64, value: u64, id: u32) -> Option<u64> {
    if key as u32 == id {
        return Some(value);
    }
    if key == HOLE && is_cell(value) {
        for i in 0..POLY_COUNT {
            let at = POLY_CASES + POLY_CASE_SIZE * i;
            // SAFETY: `value` is a live POLY_IC cell (4 cases).
            let (k, h) = unsafe { (word(value, at), word(value, at + 8)) };
            if k as u32 == id {
                return Some(h);
            }
        }
    }
    None
}

/// The ShapeId of the cell `v` (word0 bytes 0-3).
///
/// # Safety
/// `v` is a live cell of an isolate that runs nothing.
pub unsafe fn shape_id(v: u64) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { word(v, 0) as u32 }
}

/// The kind byte of the cell `v`.
///
/// # Safety
/// As [`shape_id`].
pub unsafe fn kind(v: u64) -> u8 {
    // SAFETY: the caller's contract.
    (unsafe { word(v, 0) } >> (8 * KIND_BYTE)) as u8
}

/// The int32 of an int32 Value, if it is one.
pub const fn int32_of(bits: u64) -> Option<i32> {
    if bits & NUMBER_TAG == NUMBER_TAG {
        Some(bits as u32 as i32)
    } else {
        None
    }
}

/// The int32 Value of `n`.
pub const fn int32_bits(n: i32) -> u64 {
    NUMBER_TAG | n as u32 as u64
}

/// The double Value of `x`.
pub fn double_bits(x: f64) -> u64 {
    x.to_bits().wrapping_add(DOUBLE_ENCODE_OFFSET)
}

/// The probe script of [`check`]: an object with two fields and an array, read by a
/// function that runs past the feedback threshold, so its named-load IC holds a case.
pub const CHECK_SRC: &str = "\
function LayoutProbe() { this.a = 7; this.b = [11, 22, 33]; }\n\
function layoutGet(o) { return o.b; }\n\
function layoutLen(a) { return a.length; }\n\
var layoutObj = new LayoutProbe();\n\
for (var layoutI = 0; layoutI < 20; layoutI++) { layoutGet(layoutObj); layoutLen(layoutObj.b); }\n";

/// Checks every copied layout fact on live objects of `session` (runs [`CHECK_SRC`] there).
pub fn check(session: &mut crate::harness::Session) -> Result<String, String> {
    mirror_agrees()?;
    let r = session.run(CHECK_SRC, "layout-probe.js");
    r.result.map_err(|e| format!("layout probe: {e}"))?;
    let o = session.function_value("layoutObj")?.to_bits();
    let a = session.function_value("layoutObj.b")?.to_bits();
    let f = session.function_value("layoutGet")?;
    let info = session.input_of(f)?.key;
    let fl = session.function_value("layoutLen")?;
    let info_len = session.input_of(fl)?.key;
    if !is_cell(o) || !is_cell(a) {
        return Err("probe values are not cells".into());
    }
    // SAFETY: `o` and `a` are live cells held by globals of the session's isolate, `info` is
    // the FunctionInfo of a live function, and the isolate runs nothing while this reads.
    unsafe {
        if kind(o) != KIND_OBJECT || kind(a) != KIND_ARRAY {
            return Err(format!("kind bytes {} and {}", kind(o), kind(a)));
        }
        if word(o, OBJECT_FIELDS) != int32_bits(7) || word(o, OBJECT_FIELDS + 8) != a {
            return Err("in-object fields are not at +24, +32".into());
        }
        if int32_of(word(a, ARRAY_LENGTH)) != Some(3) {
            return Err("array length is not an int32 at +24".into());
        }
        let e = word(a, ARRAY_ELEMENTS);
        if !is_cell(e) || int32_of(word(e, ELEMENTS_CAPACITY)).is_none_or(|c| c < 3) {
            return Err("ELEMENTS cell or its capacity".into());
        }
        for (i, want) in [11, 22, 33].into_iter().enumerate() {
            if word(e, ELEMENTS_DATA + 8 * i as u32) != int32_bits(want) {
                return Err(format!("element {i} is not at +16 + 8i"));
            }
        }
        let cell = feedback_cell(info)?.ok_or("layoutGet has no feedback vector")?;
        // Slot 0 of layoutGet is the LoadNamed site `o.b`: field 1 of LayoutProbe objects.
        let (k, v) = feedback_slot(cell, 0);
        let h = ic_handler_for(k, v, shape_id(o)).ok_or("no IC case for the probe's shape")?;
        if h != field_in_object_handler(OBJECT_FIELDS + 8) {
            return Err(format!(
                "IC handler {h:#x}, not FIELD_IN_OBJECT at +32 ({:#x})",
                field_in_object_handler(OBJECT_FIELDS + 8)
            ));
        }
        let cell = feedback_cell(info_len)?.ok_or("layoutLen has no feedback vector")?;
        let (k, v) = feedback_slot(cell, 0);
        let h = ic_handler_for(k, v, shape_id(a)).ok_or("no IC case for the array's shape")?;
        if h != ARRAY_LENGTH_HANDLER {
            return Err(format!(
                "array length IC handler {h:#x}, not {ARRAY_LENGTH_HANDLER:#x}"
            ));
        }
    }
    Ok(format!(
        "objects, arrays, ELEMENTS, the named-load IC and FunctionInfo.feedback (+{FI_FEEDBACK}) as copied"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_b::harness::Config;

    #[test]
    fn layout_copies_hold() {
        let mut s = crate::harness::Session::new(&Config::default()).expect("session");
        let note = check(&mut s).expect("layout");
        assert!(note.contains("FunctionInfo.feedback"), "{note}");
    }

    #[test]
    fn handler_encoding() {
        assert_eq!(field_in_object_handler(24), NUMBER_TAG | (1 | (24 << 4)));
        assert_eq!(int32_of(int32_bits(-5)), Some(-5));
        assert_eq!(int32_of(double_bits(1.5)), None);
        assert!(!is_cell(int32_bits(0)) && !is_cell(HOLE) && !is_cell(0));
    }
}
