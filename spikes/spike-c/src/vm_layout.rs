//! The VM layout facts the emitted code reads, taken from the `ep9_vm::layout` export (M4-P3
//! KEYED, 8140b68; SPIKEC-4 switched to it, decision P3-ORCH-9). Until SPIKEC-4 this module
//! kept hand copies of them; now every offset, kind id and handler field below is the VM's
//! own constant (the POLY_IC handler half too since M5-P0: `POLY_HANDLER`, MC-w16-4, C-18), so a
//! layout change moves the compiled code with it. [`check`] proves them on live objects before
//! any compile: a wrong export fails there.
//!
//! Each fact names the version that owns it (MC-w15-5: one owner per fact, doc 13 §7.3; the
//! C-18 `VM_LAYOUT` version covers only what no other version owns):
//!
//! | Fact | Export (`ep9_vm::layout`) | Owning version |
//! |---|---|---|
//! | word0: ShapeId in bytes 0-3, kind in byte 5 | `SHAPE_ID_BYTE`, `KIND_BYTE` | OBJECT_LAYOUT 5 |
//! | OBJECT kind 16, ARRAY kind 18 | `KIND_OBJECT`, `KIND_ARRAY` | OBJECT_LAYOUT 5 |
//! | in-object field `f` of an OBJECT (no fixed fields) at `OBJECT_FIELDS + 8f` | `OBJECT_FIELDS` | OBJECT_LAYOUT 5 |
//! | ARRAY: elements store, length (an int32 Value) | `ARRAY_ELEMENTS`, `ARRAY_LENGTH` | OBJECT_LAYOUT 5 |
//! | the elements store read as Values is an ELEMENTS cell (kind 65); `CheckArray`, `LoadElement` and `StoreElement` exit on any other elements cell (ELEMENTS_DOUBLE 81, ELEMENTS_DICT 82; V10-D9b, M5-P0 review F1) | `KIND_ELEMENTS` | OBJECT_LAYOUT 5 |
//! | ELEMENTS: capacity (an int32 Value), values from `ELEMENTS_DATA` | `ELEMENTS_CAPACITY`, `ELEMENTS_DATA` | OBJECT_LAYOUT 5 |
//! | the hole (`Value::EMPTY`) | `HOLE` | Value encoding 1 |
//! | FEEDBACK cell: its FunctionInfo, slot `i` at `FEEDBACK_SLOTS + FEEDBACK_SLOT_SIZE * i`: key (weak), then value | `FEEDBACK_INFO`, `FEEDBACK_SLOTS`, `FEEDBACK_SLOT_SIZE`, `FEEDBACK_KEY`, `FEEDBACK_VALUE` | IC_LAYOUT 3 |
//! | smi handler: kind in bits 0-3 (FIELD_IN_OBJECT, ARRAY_LENGTH), byte offset from bit 4 | `SMI_KIND_MASK`, `SMI_SHIFT`, `SMI_FIELD_IN_OBJECT`, `SMI_ARRAY_LENGTH` | IC_LAYOUT 3 |
//! | POLY_IC: case `i` at `POLY_CASES + POLY_CASE_SIZE * i`: key (weak), then handler at `+ POLY_HANDLER` (MC-w16-4); `POLY_COUNT` cases | `POLY_CASES`, `POLY_CASE_SIZE`, `POLY_HANDLER`, `POLY_COUNT` | IC_LAYOUT 3 |
//! | `FunctionInfo.feedback`: a pointer to the unit slot that holds the FEEDBACK cell (null: none) | `FI_FEEDBACK` | VM_LAYOUT 1 (C-18) |
//!
//! Since M5-P0 the export is `ep9_vm::layout`, in every ep9_vm build (C-18). What is left here
//! is derived from it: the two smi handlers the shape guards compare and the Value helpers.

use ep9_value::encoding::{DOUBLE_ENCODE_OFFSET, NOT_CELL_MASK, NUMBER_TAG};

pub use ep9_vm::layout::{
    ARRAY_ELEMENTS, ARRAY_LENGTH, ELEMENTS_CAPACITY, ELEMENTS_DATA, FEEDBACK_INFO, FEEDBACK_KEY,
    FEEDBACK_SLOT_SIZE, FEEDBACK_SLOTS, FEEDBACK_VALUE, FI_FEEDBACK, HOLE, KIND_ARRAY, KIND_BYTE,
    KIND_ELEMENTS, KIND_OBJECT, OBJECT_FIELDS, POLY_CASE_SIZE, POLY_CASES, POLY_COUNT,
    POLY_HANDLER, SHAPE_ID_BYTE, SMI_ARRAY_LENGTH, SMI_FIELD_IN_OBJECT, SMI_KIND_MASK, SMI_SHIFT,
};

/// The smi handler of `length` on an Array shape (an int32 Value of kind ARRAY_LENGTH).
pub const ARRAY_LENGTH_HANDLER: u64 = NUMBER_TAG | SMI_ARRAY_LENGTH as u32 as u64;

/// The smi handler (an int32 Value) of an in-object field at byte offset `offset`.
pub const fn field_in_object_handler(offset: u32) -> u64 {
    NUMBER_TAG | (SMI_FIELD_IN_OBJECT | ((offset as i32) << SMI_SHIFT)) as u32 as u64
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
/// [`FI_FEEDBACK`], checked by the cell's back pointer ([`FEEDBACK_INFO`]: the FunctionInfo
/// as a native word). `None` if the function has no vector yet.
///
/// # Safety
/// `info` is the address of a live FunctionInfo (`NativeTarget::info`), and its isolate runs
/// nothing while this reads.
pub unsafe fn feedback_cell(info: usize) -> Result<Option<u64>, String> {
    // SAFETY: `info` is a live FunctionInfo (caller); FI_FEEDBACK is the offset of a field
    // inside it (`offset_of!` in ep9_vm).
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
    // SAFETY: `cell` is the live FEEDBACK cell; FEEDBACK_INFO is its FunctionInfo word.
    let back = unsafe { word(cell, FEEDBACK_INFO) };
    if back != info as u64 + ep9_vm::layout::NATIVE_WORD_OFFSET {
        return Err(format!(
            "FEEDBACK cell +{FEEDBACK_INFO} is {back:#x}, not the FunctionInfo {info:#x} (layout drift)"
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
    unsafe {
        (
            word(cell, at + FEEDBACK_KEY),
            word(cell, at + FEEDBACK_VALUE),
        )
    }
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
            // SAFETY: `value` is a live POLY_IC cell (POLY_COUNT cases).
            let (k, h) = unsafe { (word(value, at), word(value, at + POLY_HANDLER)) };
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
    unsafe { word(v, SHAPE_ID_BYTE) as u32 }
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

/// Proves every exported fact the emitter reads on live objects of `session` (runs
/// [`CHECK_SRC`] there).
pub fn check(session: &mut crate::harness::Session) -> Result<String, String> {
    let r = session.run(CHECK_SRC, "layout-probe.js");
    r.result.map_err(|e| format!("layout probe: {e}"))?;
    check_versions(VM_VERSIONS)?;
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
            return Err(format!(
                "in-object fields are not at +{OBJECT_FIELDS}, +{}",
                OBJECT_FIELDS + 8
            ));
        }
        if int32_of(word(a, ARRAY_LENGTH)) != Some(3) {
            return Err(format!("array length is not an int32 at +{ARRAY_LENGTH}"));
        }
        let e = word(a, ARRAY_ELEMENTS);
        if !is_cell(e) || int32_of(word(e, ELEMENTS_CAPACITY)).is_none_or(|c| c < 3) {
            return Err("ELEMENTS cell or its capacity".into());
        }
        if kind(e) != KIND_ELEMENTS {
            return Err(format!("the array's elements cell has kind {}", kind(e)));
        }
        for (i, want) in [11, 22, 33].into_iter().enumerate() {
            if word(e, ELEMENTS_DATA + 8 * i as u32) != int32_bits(want) {
                return Err(format!("element {i} is not at +{ELEMENTS_DATA} + 8i"));
            }
        }
        let cell = feedback_cell(info)?.ok_or("layoutGet has no feedback vector")?;
        // Slot 0 of layoutGet is the LoadNamed site `o.b`: field 1 of LayoutProbe objects.
        let (k, v) = feedback_slot(cell, 0);
        let h = ic_handler_for(k, v, shape_id(o)).ok_or("no IC case for the probe's shape")?;
        if h != field_in_object_handler(OBJECT_FIELDS + 8) {
            return Err(format!(
                "IC handler {h:#x}, not FIELD_IN_OBJECT at +{} ({:#x})",
                OBJECT_FIELDS + 8,
                field_in_object_handler(OBJECT_FIELDS + 8)
            ));
        }
        if h as i32 & SMI_KIND_MASK != SMI_FIELD_IN_OBJECT {
            return Err(format!("IC handler {h:#x}: kind bits"));
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
        "objects, arrays, ELEMENTS, the named-load IC and FunctionInfo.feedback (+{FI_FEEDBACK}) \
         as ep9_vm::layout exports them; versions {}",
        PINNED
            .iter()
            .map(|(n, v)| format!("{n} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The versions this module's facts (the table above) and the emitter's frame and exit code
/// were written against (SPIKEC-5). A VM that bumps one of them changed a fact compiled code
/// bakes in, so [`check`] refuses until the spike is moved to it.
pub const PINNED: [(&str, u32); 4] = [
    ("OBJECT_LAYOUT", 5),
    ("IC_LAYOUT", 3),
    ("FRAME", 2),
    ("VM_LAYOUT", 1),
];

/// The VM's own versions, in [`PINNED`] order.
const VM_VERSIONS: [u32; 4] = {
    use ep9_vm::versions::*;
    [
        OBJECT_LAYOUT_VERSION,
        IC_LAYOUT_VERSION,
        FRAME_VERSION,
        VM_LAYOUT_VERSION,
    ]
};

/// Refuses if a version in `vm` (in [`PINNED`] order) is not the pinned one.
fn check_versions(vm: [u32; 4]) -> Result<(), String> {
    for ((name, want), got) in PINNED.iter().zip(vm) {
        if got != *want {
            return Err(format!(
                "{name} is {got} in ep9_vm, the spike is pinned to {want}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_b::harness::Config;

    #[test]
    fn layout_export_holds() {
        let mut s = crate::harness::Session::new(&Config::default()).expect("session");
        let note = check(&mut s).expect("layout");
        assert!(note.contains("FunctionInfo.feedback"), "{note}");
        assert!(note.contains("VM_LAYOUT 1"), "{note}");
    }

    /// The pin refuses a VM with any other version.
    #[test]
    fn a_bumped_version_is_refused() {
        assert_eq!(check_versions(VM_VERSIONS), Ok(()));
        for i in 0..4 {
            let mut v = VM_VERSIONS;
            v[i] += 1;
            let e = check_versions(v).expect_err("refused");
            assert!(e.contains(PINNED[i].0), "{e}");
        }
    }

    /// The values the emitter baked before the switch (the M4-P0 hand copies) are the
    /// export's values today: switching changed no byte of compiled code. A later layout
    /// change fails here and in [`check`], and the emitter follows the export.
    #[test]
    fn export_equals_the_old_copies() {
        assert_eq!(
            (SHAPE_ID_BYTE, KIND_BYTE, KIND_OBJECT, KIND_ARRAY),
            (0, 5, 16, 18)
        );
        assert_eq!((OBJECT_FIELDS, ARRAY_ELEMENTS, ARRAY_LENGTH), (24, 16, 24));
        assert_eq!((ELEMENTS_CAPACITY, ELEMENTS_DATA), (8, 16));
        assert_eq!(HOLE, ep9_vm::Value::EMPTY.to_bits());
        assert_eq!(
            (
                FEEDBACK_INFO,
                FEEDBACK_SLOTS,
                FEEDBACK_SLOT_SIZE,
                FEEDBACK_KEY,
                FEEDBACK_VALUE
            ),
            (16, 24, 16, 0, 8)
        );
        assert_eq!((POLY_CASES, POLY_CASE_SIZE, POLY_COUNT), (8, 16, 4));
        // M5-P0 (C-18): the handler half, a literal + 8 before, and the ELEMENTS kind.
        assert_eq!((POLY_HANDLER, KIND_ELEMENTS), (8, 65));
        assert_eq!(
            (
                SMI_KIND_MASK,
                SMI_SHIFT,
                SMI_FIELD_IN_OBJECT,
                SMI_ARRAY_LENGTH
            ),
            (0xF, 4, 1, 3)
        );
    }

    #[test]
    fn handler_encoding() {
        assert_eq!(field_in_object_handler(24), NUMBER_TAG | (1 | (24 << 4)));
        assert_eq!(int32_of(int32_bits(-5)), Some(-5));
        assert_eq!(int32_of(double_bits(1.5)), None);
        assert!(!is_cell(int32_bits(0)) && !is_cell(HOLE) && !is_cell(0));
    }
}
