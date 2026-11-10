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
//! | word0: ShapeId in bytes 0-3, kind in byte 5 | `SHAPE_ID_BYTE`, `KIND_BYTE` | OBJECT_LAYOUT 7 |
//! | OBJECT kind 16, ARRAY kind 18 | `KIND_OBJECT`, `KIND_ARRAY` | OBJECT_LAYOUT 7 |
//! | in-object field `f` of an OBJECT (no fixed fields) at `OBJECT_FIELDS + 8f` | `OBJECT_FIELDS` | OBJECT_LAYOUT 7 |
//! | ARRAY: elements store, length (an int32 Value below 2^31, a double Value from 2^31 on, M5-P2 ELEM2; only a DICTIONARY array is that long, and `CheckArray` exits on it, so `LoadElement` and `StoreElement` read the low 32 bits as the dense bound) | `ARRAY_ELEMENTS`, `ARRAY_LENGTH` | OBJECT_LAYOUT 7 |
//! | the two elements storage forms the element ops read: an ELEMENTS cell (kind 65, Values) or an ELEMENTS_DOUBLE cell (kind 81, raw f64); `LoadElement` and `StoreElement` exit on any other elements store (an ELEMENTS_DICT cell, 82; V10-D9b, M5-P0 review F1) | `KIND_ELEMENTS`, `KIND_ELEMENTS_DOUBLE`, `KIND_ELEMENTS_DICT` | OBJECT_LAYOUT 7 |
//! | ELEMENTS: capacity (an int32 Value), values from `ELEMENTS_DATA` | `ELEMENTS_CAPACITY`, `ELEMENTS_DATA` | OBJECT_LAYOUT 7 |
//! | ELEMENTS_DOUBLE: capacity (an int32 Value), raw f64 from `ELEMENTS_DOUBLE_DATA`; the hole is a NaN no store writes; a store writes every NaN as the pure NaN (`PURE_NAN_BITS`, proved below) | `ELEMENTS_DOUBLE_CAPACITY`, `ELEMENTS_DOUBLE_DATA`, `ELEMENTS_DOUBLE_HOLE` | OBJECT_LAYOUT 7 |
//! | a shape's elements kind: `counts` bits 28-31; kinds 0-5 hold elements in the two forms above (INT32 and ELEMENTS kinds: Values; DOUBLE kinds: raw f64), 6 is DICTIONARY, 7, 8 the TypedArray views (OBJECT_LAYOUT 9), 9-15 reserved. `CheckArray` (the kind guard, M5-P1-ORCH-11) reads it from the receiver's shape and exits unless it is in [`ARRAY_KINDS`] | `SHAPE_ELEMENTS_KIND_SHIFT`, `ELEMENTS_KIND_*` | OBJECT_LAYOUT 7 |
//! | a shape's `flags` word (an int32 Value) at `SHAPE_FLAGS`: bit 4 SEALED, bit 5 FROZEN (always with SEALED; H18 "extra element kinds"). A level sits on a dense kind, so `CheckArray` also exits when either bit is set (M7-P3: a dense kind no longer means writable elements). Proved on a live sealed and a live frozen Array by `ep9_vm` `tests::layout6` | `SHAPE_FLAGS`, `SHAPE_FLAG_SEALED`, `SHAPE_FLAG_FROZEN` | OBJECT_LAYOUT 8 |
//! | a cell's shape is the cell `(address & !(SHAPE_CAGE_ALIGN - 1)) \| ShapeId` (kind `KIND_SHAPE`), its `counts` word is at `SHAPE_COUNTS` (proved below on live arrays of each kind); the CONTEXT kind id (the `LoadContextSlot` guard; proved on a live closure). Spike C hand facts until M6-P0 | `SHAPE_CAGE_ALIGN`, `SHAPE_COUNTS`, `KIND_SHAPE`, `KIND_CONTEXT` | OBJECT_LAYOUT 7 |
//! | the hole (`Value::EMPTY`) | `HOLE` | Value encoding 1 |
//! | FEEDBACK cell: its FunctionInfo, slot `i` at `FEEDBACK_SLOTS + FEEDBACK_SLOT_SIZE * i`: key (weak), then value | `FEEDBACK_INFO`, `FEEDBACK_SLOTS`, `FEEDBACK_SLOT_SIZE`, `FEEDBACK_KEY`, `FEEDBACK_VALUE` | IC_LAYOUT 4 |
//! | the key half of an IC entry (a feedback slot or a POLY_IC case) that no site filled: `undefined`; the shape guard's "unfilled" exit compares with it (a hand fact until M6-P0) | `VALUE_UNDEFINED` | IC_LAYOUT 4 |
//! | smi handler: kind in bits 0-3 (FIELD_IN_OBJECT, ARRAY_LENGTH), byte offset from bit 4 | `SMI_KIND_MASK`, `SMI_SHIFT`, `SMI_FIELD_IN_OBJECT`, `SMI_ARRAY_LENGTH` | IC_LAYOUT 4 |
//! | an out-of-line field: the object's SLOTS cell at `OBJECT_SLOTS`, field `k` past the in-object capacity at `SLOTS_DATA + 8k` of it, and its smi handler kind `SMI_FIELD_OUT_OF_LINE` (the named load of SPIKEC-9; proved below on a live object). Spike C hand facts (`HAND_FACTS_M6`) from M6-P1 until M7-P0 (MC-P1-LAYOUTX) | `OBJECT_SLOTS`, `SLOTS_DATA`, `SMI_FIELD_OUT_OF_LINE` | OBJECT_LAYOUT 7 (the first two), IC_LAYOUT 4 (the handler kind) |
//! | POLY_IC: case `i` at `POLY_CASES + POLY_CASE_SIZE * i`: key (weak), then handler at `+ POLY_HANDLER` (MC-w16-4); `POLY_COUNT` cases | `POLY_CASES`, `POLY_CASE_SIZE`, `POLY_HANDLER`, `POLY_COUNT` | IC_LAYOUT 4 |
//! | `FunctionInfo.feedback`: a pointer to the unit slot that holds the FEEDBACK cell (null: none) | `FI_FEEDBACK` | VM_LAYOUT 1 (C-18) |
//! | the shared exit's register dump below sp: word n = xn for x0-x30, 16-aligned (a hand fact until M6-P0; the emitter's `deopt_exit`) | `DUMP_BYTES` | DEOPT_META 1 (0 until A2 start) |
//!
//! Since M5-P0 the export is `ep9_vm::layout`, in every ep9_vm build (C-18). What is left here
//! is derived from it: the two smi handlers the shape guards compare and the Value helpers.
//! Since M6-P0 every fact the spike bakes is an export: the 6 facts the month-5 export lacked
//! (the shape address rule, the shape's `counts` offset, the SHAPE and CONTEXT kind ids, the
//! unfilled IC key and the register dump; `HAND_FACTS` until M6-P0) are `ep9_vm::layout` items
//! (plan v11 §2.4 step 3 and §2.6 task 5; v11in #2, MC-w23-9), so no hand fact is left.
//! [`check`] still proves the shape rule and the kind ids on live objects.

use ep9_value::encoding::{DOUBLE_ENCODE_OFFSET, NOT_CELL_MASK, NUMBER_TAG};

pub use ep9_vm::layout::{
    ARRAY_ELEMENTS, ARRAY_LENGTH, ELEMENTS_CAPACITY, ELEMENTS_DATA, ELEMENTS_DOUBLE_CAPACITY,
    ELEMENTS_DOUBLE_DATA, ELEMENTS_DOUBLE_HOLE, ELEMENTS_KIND_DICTIONARY,
    ELEMENTS_KIND_HOLEY_DOUBLE, ELEMENTS_KIND_HOLEY_ELEMENTS, ELEMENTS_KIND_HOLEY_INT32,
    ELEMENTS_KIND_PACKED_DOUBLE, ELEMENTS_KIND_PACKED_ELEMENTS, ELEMENTS_KIND_PACKED_INT32,
    FEEDBACK_INFO, FEEDBACK_KEY, FEEDBACK_SLOT_SIZE, FEEDBACK_SLOTS, FEEDBACK_VALUE, FI_FEEDBACK,
    HOLE, KIND_ARRAY, KIND_BYTE, KIND_CONTEXT, KIND_ELEMENTS, KIND_ELEMENTS_DICT,
    KIND_ELEMENTS_DOUBLE, KIND_OBJECT, KIND_SHAPE, OBJECT_FIELDS, OBJECT_SLOTS, POLY_CASE_SIZE,
    POLY_CASES, POLY_COUNT, POLY_HANDLER, SHAPE_CAGE_ALIGN, SHAPE_COUNTS,
    SHAPE_ELEMENTS_KIND_SHIFT, SHAPE_FLAG_FROZEN, SHAPE_FLAG_SEALED, SHAPE_FLAGS, SHAPE_ID_BYTE,
    SLOTS_DATA, SMI_ARRAY_LENGTH, SMI_FIELD_IN_OBJECT, SMI_FIELD_OUT_OF_LINE, SMI_KIND_MASK,
    SMI_SHIFT,
};

/// The elements kinds whose elements the element ops read and write (bit `k` = kind `k`):
/// PACKED and HOLEY INT32, DOUBLE and ELEMENTS. The kind guard (`CheckArray`) exits on any
/// other kind: DICTIONARY (an ELEMENTS_DICT cell, fail closed) and the reserved values 7-15.
pub const ARRAY_KINDS: u16 = 1 << ELEMENTS_KIND_PACKED_INT32
    | 1 << ELEMENTS_KIND_HOLEY_INT32
    | 1 << ELEMENTS_KIND_PACKED_DOUBLE
    | 1 << ELEMENTS_KIND_HOLEY_DOUBLE
    | 1 << ELEMENTS_KIND_PACKED_ELEMENTS
    | 1 << ELEMENTS_KIND_HOLEY_ELEMENTS;

/// True for a DOUBLE elements kind (its elements are raw f64 in an ELEMENTS_DOUBLE cell).
pub const fn is_double_kind(k: u8) -> bool {
    k == ELEMENTS_KIND_PACKED_DOUBLE || k == ELEMENTS_KIND_HOLEY_DOUBLE
}

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

/// The shape cell of the cell `v` by the shape rule ([`SHAPE_CAGE_ALIGN`]).
///
/// # Safety
/// As [`shape_id`].
pub unsafe fn shape_of(v: u64) -> u64 {
    // SAFETY: the caller's contract.
    (v & !(SHAPE_CAGE_ALIGN - 1)) | u64::from(unsafe { shape_id(v) })
}

/// The elements kind (counts bits 28-31) of the shape of the cell `v`, as the kind guard reads
/// it.
///
/// # Safety
/// As [`shape_id`].
pub unsafe fn elements_kind(v: u64) -> u8 {
    // SAFETY: the caller's contract; [`check`] proved the rule and the offset.
    let counts = unsafe { word(shape_of(v), SHAPE_COUNTS) } as u32;
    (counts >> SHAPE_ELEMENTS_KIND_SHIFT) as u8
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

/// The Value of the number `x` as `Value::number` gives it (SPIKEC-9: a pool constant in a
/// deopt record): an int32 Value for an integer in int32 range that is not -0, else the
/// double Value with every NaN as the pure NaN.
pub fn number_bits(x: f64) -> u64 {
    ep9_value::Value::number(x).to_bits()
}

/// The probe script of [`check`]: an object with two fields and an array, read by a
/// function that runs past the feedback threshold, so its named-load IC holds a case.
pub const CHECK_SRC: &str = "\
function LayoutProbe() { this.a = 7; this.b = [11, 22, 33]; }\n\
function layoutGet(o) { return o.b; }\n\
function layoutLen(a) { return a.length; }\n\
var layoutObj = new LayoutProbe();\n\
for (var layoutI = 0; layoutI < 20; layoutI++) { layoutGet(layoutObj); layoutLen(layoutObj.b); }\n\
var layoutKinds = [[1, 2, 3], [1, , 3], [1.5, 2.5, 3.5], [1.5, , 3.5], ['a', 2, 3], ['a', , 3]];\n\
layoutKinds[2][1] = NaN;\n";

/// The elements kind of each array of `layoutKinds` in [`CHECK_SRC`] (elements 0 and 2 are
/// 1 and 3, or 1.5 and 3.5; element 1 is 2, NaN, or a hole).
const CHECK_KINDS: [u8; 6] = [
    ELEMENTS_KIND_PACKED_INT32,
    ELEMENTS_KIND_HOLEY_INT32,
    ELEMENTS_KIND_PACKED_DOUBLE,
    ELEMENTS_KIND_HOLEY_DOUBLE,
    ELEMENTS_KIND_PACKED_ELEMENTS,
    ELEMENTS_KIND_HOLEY_ELEMENTS,
];

/// Proves every exported fact the emitter reads on live objects of a private, fresh session
/// of `config` (runs [`CHECK_SRC`] there): no other script ran in it, so no probe global is
/// pre-set (Codex M5-P3 r2 spk 1).
pub fn check(config: &spike_b::harness::Config) -> Result<String, String> {
    check_in(&mut crate::harness::Session::new(config)?)
}

/// [`check`] on `session`. Each array the probes read is checked before its raw words are
/// read (kind byte, int32 length, an elements cell of the right kind, its capacity), so a
/// pre-set array global gives an `Err`; the other probe globals are trusted because
/// [`check`] gives a fresh session.
fn check_in(session: &mut crate::harness::Session) -> Result<String, String> {
    let r = session.run(CHECK_SRC, "layout-probe.js");
    r.result.map_err(|e| format!("layout probe: {e}"))?;
    check_versions(VM_VERSIONS)?;
    // SAFETY: the session ran CHECK_SRC just now (check_elem2 is safe: it runs ELEM2_SRC).
    unsafe { check_kinds(session) }.and_then(|()| check_elem2(session))?;
    check_context(session)?;
    check_slots(session)?;
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
        if !is_cell(e) || kind(e) != KIND_ELEMENTS {
            return Err(format!(
                "the array's elements word {e:#x} is no ELEMENTS cell"
            ));
        }
        if int32_of(word(e, ELEMENTS_CAPACITY)).is_none_or(|c| c < 3) {
            return Err("the ELEMENTS cell's capacity".into());
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
         as ep9_vm::layout exports them; the elements kind in the shape and both storage forms \
         on arrays of the 6 kinds, DICTIONARY (3 ways) and site-born ones; a closure's CONTEXT cell; an out-of-line field and its IC handler (the 3 out-of-line exports, hand facts until M7-P0); versions {}",
        PINNED
            .iter()
            .map(|(n, v)| format!("{n} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The kind-guard facts on the six `layoutKinds` arrays of [`CHECK_SRC`]: the shape that the
/// shape rule finds is a SHAPE cell, its `counts` bits 28-31 are the array's elements kind
/// ([`CHECK_KINDS`]), the elements cell has the storage form of that kind, and the data words
/// are where the export says: Values at `ELEMENTS_DATA`, raw f64 at `ELEMENTS_DOUBLE_DATA`, the
/// two holes, and a stored NaN as the pure NaN.
///
/// # Safety
/// The session ran [`CHECK_SRC`]. Between its `function_value` runs, nothing runs while this
/// reads.
unsafe fn check_kinds(session: &mut crate::harness::Session) -> Result<(), String> {
    use ep9_value::encoding::PURE_NAN_BITS;
    for (i, want) in CHECK_KINDS.into_iter().enumerate() {
        let a = session
            .function_value(&format!("layoutKinds[{i}]"))?
            .to_bits();
        if !is_cell(a) {
            return Err(format!("layoutKinds[{i}] is not a cell"));
        }
        // SAFETY: `a` is a live cell held by a global of the session's isolate, which runs
        // nothing while this reads; checked an Array of length 3 (kind byte, int32 length)
        // before its shape cell (the shape rule, checked by its kind byte) and its elements
        // word (checked a cell of the kind's storage form before it is read) are read.
        unsafe {
            if kind(a) != KIND_ARRAY || int32_of(word(a, ARRAY_LENGTH)) != Some(3) {
                return Err(format!("layoutKinds[{i}] is no Array of length 3"));
            }
            let shape = shape_of(a);
            if kind(shape) != KIND_SHAPE {
                return Err(format!(
                    "layoutKinds[{i}]: the cell at (address & !{SHAPE_CAGE_ALIGN:#x}) | ShapeId has kind {}, not SHAPE",
                    kind(shape)
                ));
            }
            if int32_of(word(shape, SHAPE_COUNTS)).is_none() {
                return Err(format!("shape +{SHAPE_COUNTS} is not an int32 Value"));
            }
            let k = elements_kind(a);
            if k != want {
                return Err(format!(
                    "layoutKinds[{i}]: counts bits {SHAPE_ELEMENTS_KIND_SHIFT}-31 give kind {k}, not {want}"
                ));
            }
            let e = word(a, ARRAY_ELEMENTS);
            let holey = i % 2 == 1;
            if !is_cell(e) {
                return Err(format!("layoutKinds[{i}]: elements word {e:#x} is no cell"));
            }
            if is_double_kind(k) {
                if kind(e) != KIND_ELEMENTS_DOUBLE
                    || int32_of(word(e, ELEMENTS_DOUBLE_CAPACITY)).is_none_or(|c| c < 3)
                {
                    return Err(format!("kind {k}: no ELEMENTS_DOUBLE cell or capacity"));
                }
                let at = |j: u32| word(e, ELEMENTS_DOUBLE_DATA + 8 * j);
                let one = if holey {
                    ELEMENTS_DOUBLE_HOLE
                } else {
                    PURE_NAN_BITS
                };
                if (at(0), at(1), at(2)) != (1.5f64.to_bits(), one, 3.5f64.to_bits()) {
                    return Err(format!(
                        "kind {k}: raw f64 words {:#x} {:#x} {:#x} (hole {ELEMENTS_DOUBLE_HOLE:#x}, NaN {PURE_NAN_BITS:#x})",
                        at(0),
                        at(1),
                        at(2)
                    ));
                }
            } else {
                if kind(e) != KIND_ELEMENTS
                    || int32_of(word(e, ELEMENTS_CAPACITY)).is_none_or(|c| c < 3)
                {
                    return Err(format!("kind {k}: no ELEMENTS cell or capacity"));
                }
                let at = |j: u32| word(e, ELEMENTS_DATA + 8 * j);
                let one = if holey { HOLE } else { int32_bits(2) };
                if at(1) != one || at(2) != int32_bits(3) {
                    return Err(format!("kind {k}: Values {:#x} {:#x}", at(1), at(2)));
                }
            }
        }
    }
    if PURE_NAN_BITS == ELEMENTS_DOUBLE_HOLE || ARRAY_KINDS & (1 << ELEMENTS_KIND_DICTIONARY) != 0 {
        return Err("the pure NaN is the DOUBLE hole, or DICTIONARY is an array kind".into());
    }
    Ok(())
}

/// The versions this module's facts (the table above) and the emitter's frame and exit code
/// were written against (SPIKEC-5). A VM that bumps one of them changed a fact compiled code
/// bakes in, so [`check`] refuses until the spike is moved to it. Value encoding (since M5-P5,
/// MODES-CHECK-w22 MC-w22-11) owns `HOLE`, `NATIVE_WORD_OFFSET` and the [`ENCODING_READS`]
/// constants, which the emitter and the graph builder bake into code. DEOPT_META (since M6-P0;
/// MC-w23-9, V11-D12) owns the H29 byte form [`crate::deopt_meta`] writes and the register dump
/// the shared exit stores (`DUMP_BYTES`): 0 in month 6, 1 from A2 start (M7-P1, format 1 with no
/// byte changed); the spike moved with it.
pub const PINNED: [(&str, u32); 6] = [
    ("OBJECT_LAYOUT", 9),
    ("IC_LAYOUT", 4),
    ("FRAME", 2),
    ("VM_LAYOUT", 1),
    ("Value encoding", 1),
    ("DEOPT_META", 1),
];

/// The VM's own versions, in [`PINNED`] order (Value encoding is `ep9_value`'s).
const VM_VERSIONS: [u32; 6] = {
    use ep9_vm::versions::*;
    [
        OBJECT_LAYOUT_VERSION,
        IC_LAYOUT_VERSION,
        FRAME_VERSION,
        VM_LAYOUT_VERSION,
        ep9_value::encoding::VERSION,
        DEOPT_META_VERSION,
    ]
};

/// Refuses if a version in `vm` (in [`PINNED`] order) is not the pinned one.
fn check_versions(vm: [u32; 6]) -> Result<(), String> {
    match PINNED.iter().zip(vm).find(|((_, want), got)| got != want) {
        None => Ok(()),
        Some(((name, want), got)) => Err(format!("{name} is {got}, the spike is pinned to {want}")),
    }
}

/// The probe of [`check_context`]: a closure whose function's context holds one slot.
const CONTEXT_SRC: &str = "\
function layoutCtxMake() { var layoutC = 5; return function layoutCtxGet() { return layoutC; }; }\n\
var layoutCtxFn = layoutCtxMake();\n";

/// The context facts the `LoadContextSlot` guard reads, on a live closure: its function's
/// context (`layout::FUNCTION_CONTEXT`) is a cell of kind [`KIND_CONTEXT`] whose length
/// (`layout::CONTEXT_LENGTH`, an int32 Value) is 1 and whose slot 0 (`layout::CONTEXT_SLOTS`)
/// holds 5.
fn check_context(session: &mut crate::harness::Session) -> Result<(), String> {
    use ep9_vm::layout::{CONTEXT_LENGTH, CONTEXT_SLOTS, FUNCTION_CONTEXT};
    let r = session.run(CONTEXT_SRC, "layout-context.js");
    r.result.map_err(|e| format!("context probe: {e}"))?;
    let f = session.function_value("layoutCtxFn")?.to_bits();
    if !is_cell(f) {
        return Err("layoutCtxFn is not a cell".into());
    }
    // SAFETY: `f` is a live function held by a global of the session's isolate, which runs
    // nothing while this reads; its context cell is live too (checked a cell before it is read).
    unsafe {
        let c = word(f, FUNCTION_CONTEXT as u32);
        if !is_cell(c) || kind(c) != KIND_CONTEXT {
            return Err(format!(
                "the closure's context {c:#x} is not a CONTEXT cell"
            ));
        }
        let (len, slot0) = (int32_of(word(c, CONTEXT_LENGTH)), word(c, CONTEXT_SLOTS));
        if len != Some(1) || slot0 != int32_bits(5) {
            return Err(format!(
                "CONTEXT length {len:?} at +{CONTEXT_LENGTH}, slot 0 {slot0:#x} at +{CONTEXT_SLOTS}"
            ));
        }
    }
    Ok(())
}

/// The probe of [`check_elem2`]: DICTIONARY arrays made three ways, and an array-literal site
/// whose array went to DOUBLE, then its next array.
const ELEM2_SRC: &str = "\
var layoutDicts = [[1, 2, 3], [1, 2, 3], []];\n\
layoutDicts[0][100000] = 4;\n\
Object.defineProperty(layoutDicts[1], 'length', { writable: false });\n\
layoutDicts[2][4294967294] = 5;\n\
function layoutSite() { return [1, 2, 3]; }\n\
var layoutSiteA = null;\n\
for (var layoutJ = 0; layoutJ < 20; layoutJ++) layoutSiteA = layoutSite();\n\
layoutSiteA[1] = 2.5;\n\
var layoutSiteB = layoutSite();\n";

/// R17 re-proof after M5-P2 ELEM2 (plan v10 §5.7, SPIKEC-7), on live objects of [`ELEM2_SRC`]:
///
/// - DICTIONARY arrays, made the three ways ELEM2 makes them (an index far past the
///   capacity, a read-only `length`, a `length` past 2^31 - 1): the shape's kind bits give
///   DICTIONARY, which [`ARRAY_KINDS`] leaves out (so `CheckArray` exits: fail closed), the
///   elements store is an ELEMENTS_DICT cell (kind [`KIND_ELEMENTS_DICT`], which the element
///   ops refuse too), and its word at `ELEMENTS_CAPACITY` is the int32 0 (a dense bound test
///   that read it would miss). The long array's `length` is a double Value (only a
///   DICTIONARY array is that long; `CheckArray` exits before an element op reads the low
///   32 bits as the bound).
/// - An array born of an allocation site: after a site's array went to DOUBLE, the site's
///   next array is born PACKED_DOUBLE, in an ELEMENTS_DOUBLE cell with raw f64 data (the
///   shape is the kind source the guard reads; Spike C reads no ALLOC_SITE word).
///
/// A safe fn: it runs [`ELEM2_SRC`] itself, and the `&mut` session runs nothing else while it
/// reads the cells, between its `function_value` runs (the SAFETY notes inside).
fn check_elem2(session: &mut crate::harness::Session) -> Result<(), String> {
    let r = session.run(ELEM2_SRC, "layout-elem2.js");
    r.result.map_err(|e| format!("ELEM2 probe: {e}"))?;
    for i in 0..3 {
        let a = session
            .function_value(&format!("layoutDicts[{i}]"))?
            .to_bits();
        if !is_cell(a) {
            return Err(format!("layoutDicts[{i}] is not a cell"));
        }
        // SAFETY: `a` is a live cell held by a global of the session's isolate, which runs
        // nothing while this reads; checked an Array before its shape is read, and its
        // elements word checked a cell before it is read.
        unsafe {
            if kind(a) != KIND_ARRAY {
                return Err(format!("layoutDicts[{i}] is no Array"));
            }
            let k = elements_kind(a);
            if k != ELEMENTS_KIND_DICTIONARY || ARRAY_KINDS & (1 << k) != 0 {
                return Err(format!(
                    "layoutDicts[{i}]: elements kind {k}, not DICTIONARY outside ARRAY_KINDS"
                ));
            }
            let e = word(a, ARRAY_ELEMENTS);
            if !is_cell(e) || kind(e) != KIND_ELEMENTS_DICT {
                return Err(format!(
                    "layoutDicts[{i}]: the elements store is no ELEMENTS_DICT cell"
                ));
            }
            if word(e, ELEMENTS_CAPACITY) != int32_bits(0) {
                return Err(format!(
                    "layoutDicts[{i}]: ELEMENTS_DICT +{ELEMENTS_CAPACITY} is not the int32 0"
                ));
            }
            let len = word(a, ARRAY_LENGTH);
            let want = if i == 2 { None } else { int32_of(len) };
            if i == 2 && (int32_of(len).is_some() || len != double_bits(4294967295.0)) {
                return Err(format!(
                    "layoutDicts[2]: length {len:#x}, not the double 2^32 - 1"
                ));
            }
            if i < 2 && want != Some(if i == 0 { 100001 } else { 3 }) {
                return Err(format!("layoutDicts[{i}]: length {len:#x}"));
            }
        }
    }
    let born = session.function_value("layoutSiteB")?.to_bits();
    let first = session.function_value("layoutSiteA")?.to_bits();
    if !is_cell(born) || !is_cell(first) {
        return Err("layoutSiteA or layoutSiteB is not a cell".into());
    }
    // SAFETY: as above: each is checked an Array before its shape is read, and `born`'s
    // length, elements cell (kind) and capacity are checked before its data words are read
    // (a pre-set `layoutSiteB` could give an empty array whose elements word is no cell).
    unsafe {
        if kind(first) != KIND_ARRAY || kind(born) != KIND_ARRAY {
            return Err("allocation site: layoutSiteA or layoutSiteB is no Array".into());
        }
        if elements_kind(first) != ELEMENTS_KIND_PACKED_DOUBLE
            || elements_kind(born) != ELEMENTS_KIND_PACKED_DOUBLE
        {
            return Err(format!(
                "allocation site: kinds {} (stored to) and {} (born), not PACKED_DOUBLE",
                elements_kind(first),
                elements_kind(born)
            ));
        }
        let e = word(born, ARRAY_ELEMENTS);
        if int32_of(word(born, ARRAY_LENGTH)) != Some(3)
            || !is_cell(e)
            || kind(e) != KIND_ELEMENTS_DOUBLE
            || int32_of(word(e, ELEMENTS_DOUBLE_CAPACITY)).is_none_or(|c| c < 3)
        {
            return Err(format!(
                "allocation site: the born array has no length 3 or no ELEMENTS_DOUBLE cell \
                 of capacity 3 (elements word {e:#x})"
            ));
        }
        let at = |j: u32| word(e, ELEMENTS_DOUBLE_DATA + 8 * j);
        if (at(0), at(1), at(2)) != (1f64.to_bits(), 2f64.to_bits(), 3f64.to_bits()) {
            return Err("allocation site: the born array has no raw f64 1, 2, 3".into());
        }
    }
    Ok(())
}

/// The R17 read set (plan v10 §6.1 WI-SC9 (1); an input of the M1a record): every
/// `ep9_vm::layout` item Spike C reads (the emitter, the register allocator and this
/// module), with the version that owns it (MC-w15-5). The test
/// `read_set_is_what_spike_c_reads` keeps it equal to the names the spike's sources use, and
/// `read_set_names_the_owner_of_each_item` checks each owner against the doc line of the
/// item in `crates/ep9_vm/src/layout.rs` (each export starts its doc with its owner). The two
/// smi handlers ([`ARRAY_LENGTH_HANDLER`], [`field_in_object_handler`]) are derived from the
/// IC_LAYOUT items here. Since M6-P0 the 6 former hand facts are items of the set too (v11in
/// #2), and since M7-P0 the 3 out-of-line facts of SPIKEC-9 (MC-P1-LAYOUTX): no fact the spike
/// bakes is outside it or [`ENCODING_READS`].
pub const READ_SET: [(&str, &str); 75] = [
    ("ARRAY_ELEMENTS", "OBJECT_LAYOUT"),
    ("ARRAY_LENGTH", "OBJECT_LAYOUT"),
    ("CONTEXT_LENGTH", "OBJECT_LAYOUT"),
    ("CONTEXT_SLOTS", "OBJECT_LAYOUT"),
    ("CX_BUDGET", "VM_LAYOUT"),
    ("CX_NATIVE_LIMIT", "VM_LAYOUT"),
    ("CX_OUT_FP", "VM_LAYOUT"),
    ("CX_OUT_PC", "VM_LAYOUT"),
    ("CX_RUNTIME_TABLE", "VM_LAYOUT"),
    ("CX_TABLE", "VM_LAYOUT"),
    ("DUMP_BYTES", "DEOPT_META"),
    ("ELEMENTS_CAPACITY", "OBJECT_LAYOUT"),
    ("ELEMENTS_DATA", "OBJECT_LAYOUT"),
    ("ELEMENTS_DOUBLE_CAPACITY", "OBJECT_LAYOUT"),
    ("ELEMENTS_DOUBLE_DATA", "OBJECT_LAYOUT"),
    ("ELEMENTS_DOUBLE_HOLE", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_DICTIONARY", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_HOLEY_DOUBLE", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_HOLEY_ELEMENTS", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_HOLEY_INT32", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_PACKED_DOUBLE", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_PACKED_ELEMENTS", "OBJECT_LAYOUT"),
    ("ELEMENTS_KIND_PACKED_INT32", "OBJECT_LAYOUT"),
    ("FEEDBACK_INFO", "IC_LAYOUT"),
    ("FEEDBACK_KEY", "IC_LAYOUT"),
    ("FEEDBACK_SLOTS", "IC_LAYOUT"),
    ("FEEDBACK_SLOT_SIZE", "IC_LAYOUT"),
    ("FEEDBACK_VALUE", "IC_LAYOUT"),
    ("FI_FEEDBACK", "VM_LAYOUT"),
    ("FP_CALLER_FP", "FRAME"),
    ("FP_CONTEXT", "FRAME"),
    ("FP_FUNCTION_INFO", "FRAME"),
    ("FP_RECEIVER", "FRAME"),
    ("FP_RETURN_PC", "FRAME"),
    ("FUNCTION_CONTEXT", "OBJECT_LAYOUT"),
    ("HOLE", "Value encoding"),
    ("KIND_ARRAY", "OBJECT_LAYOUT"),
    ("KIND_BYTE", "OBJECT_LAYOUT"),
    ("KIND_CONTEXT", "OBJECT_LAYOUT"),
    ("KIND_ELEMENTS", "OBJECT_LAYOUT"),
    ("KIND_ELEMENTS_DICT", "OBJECT_LAYOUT"),
    ("KIND_ELEMENTS_DOUBLE", "OBJECT_LAYOUT"),
    ("KIND_OBJECT", "OBJECT_LAYOUT"),
    ("KIND_SHAPE", "OBJECT_LAYOUT"),
    ("NATIVE_WORD_OFFSET", "Value encoding"),
    ("OBJECT_FIELDS", "OBJECT_LAYOUT"),
    ("OBJECT_SLOTS", "OBJECT_LAYOUT"),
    ("POLY_CASES", "IC_LAYOUT"),
    ("POLY_CASE_SIZE", "IC_LAYOUT"),
    ("POLY_COUNT", "IC_LAYOUT"),
    ("POLY_HANDLER", "IC_LAYOUT"),
    ("RT_CALL_ANY", "VM_LAYOUT"),
    ("RT_DEOPT", "VM_LAYOUT"),
    ("RT_GET_KEYED", "VM_LAYOUT"),
    ("RT_LOAD_GLOBAL", "VM_LAYOUT"),
    ("RT_POLL", "VM_LAYOUT"),
    ("RT_SET_KEYED", "VM_LAYOUT"),
    ("RT_STACK_OVERFLOW", "VM_LAYOUT"),
    ("RT_UNWIND", "VM_LAYOUT"),
    ("SHAPE_CAGE_ALIGN", "OBJECT_LAYOUT"),
    ("SHAPE_COUNTS", "OBJECT_LAYOUT"),
    ("SHAPE_ELEMENTS_KIND_SHIFT", "OBJECT_LAYOUT"),
    ("SHAPE_FLAGS", "OBJECT_LAYOUT"),
    ("SHAPE_FLAG_FROZEN", "OBJECT_LAYOUT"),
    ("SHAPE_FLAG_SEALED", "OBJECT_LAYOUT"),
    ("SHAPE_ID_BYTE", "OBJECT_LAYOUT"),
    ("SLOTS_DATA", "OBJECT_LAYOUT"),
    ("SMI_ARRAY_LENGTH", "IC_LAYOUT"),
    ("SMI_FIELD_IN_OBJECT", "IC_LAYOUT"),
    ("SMI_FIELD_OUT_OF_LINE", "IC_LAYOUT"),
    ("SMI_KIND_MASK", "IC_LAYOUT"),
    ("SMI_SHIFT", "IC_LAYOUT"),
    ("VALUE_UNDEFINED", "IC_LAYOUT"),
    ("fp_param", "FRAME"),
    ("fp_register", "FRAME"),
];

/// The `ep9_value::encoding` constants Spike C reads (the emitter, the graph builder, the deopt
/// metadata and this module; the emitter and the builder bake them into code), owner Value
/// encoding. They are not `ep9_vm::layout` items, so [`READ_SET`] does not list them (the
/// M5-P4 W1 reviewer F8). The test `encoding_reads_are_what_spike_c_reads` keeps this list
/// equal to the names the spike's sources use (`VERSION`, which names the owner's version, is
/// not a fact).
pub const ENCODING_READS: [&str; 8] = [
    "DOUBLE_ENCODE_OFFSET",
    "NOT_CELL_MASK",
    "NUMBER_TAG",
    "PURE_NAN_BITS",
    "VALUE_FALSE",
    "VALUE_NULL",
    "VALUE_TRUE",
    "VALUE_UNDEFINED",
];

/// The R17 read set grouped by owner, with the owner's version number (`spike-c read-set`).
pub fn read_set_lines() -> Vec<String> {
    let version = |owner: &str| match owner {
        "OBJECT_LAYOUT" => ep9_vm::versions::OBJECT_LAYOUT_VERSION.to_string(),
        "IC_LAYOUT" => ep9_vm::versions::IC_LAYOUT_VERSION.to_string(),
        "FRAME" => ep9_vm::versions::FRAME_VERSION.to_string(),
        "VM_LAYOUT" => ep9_vm::versions::VM_LAYOUT_VERSION.to_string(),
        "Value encoding" => ep9_value::encoding::VERSION.to_string(),
        "DEOPT_META" => ep9_vm::versions::DEOPT_META_VERSION.to_string(),
        _ => "?".into(),
    };
    let mut out = Vec::new();
    for owner in [
        "OBJECT_LAYOUT",
        "IC_LAYOUT",
        "FRAME",
        "VM_LAYOUT",
        "Value encoding",
        "DEOPT_META",
    ] {
        let names: Vec<&str> = READ_SET
            .iter()
            .filter(|(_, o)| *o == owner)
            .map(|(n, _)| *n)
            .collect();
        let mut line = format!("{owner} {}: {} exported", version(owner), names.len());
        if !names.is_empty() {
            line.push_str(&format!(": {}", names.join(", ")));
        }
        if owner == "Value encoding" {
            line.push_str(&format!(
                "; {} `ep9_value::encoding` constants: {}",
                ENCODING_READS.len(),
                ENCODING_READS.join(", ")
            ));
        }
        out.push(line);
    }
    out
}

// ---- out-of-line fields (SPIKEC-9) ---------------------------------------------------------

// `OBJECT_SLOTS`, `SLOTS_DATA` and `SMI_FIELD_OUT_OF_LINE` are `ep9_vm::layout` exports since
// M7-P0 (MC-P1-LAYOUTX). From M6-P1 (SPIKEC-9) until then they were the hand facts
// `HAND_FACTS_M6` here, because `layout.rs` was frozen until the m6-p5 tag (plan v11 rule 2).

/// The smi handler (an int32 Value) of an out-of-line field at byte offset `offset` of the
/// SLOTS cell.
pub const fn field_out_of_line_handler(offset: u32) -> u64 {
    NUMBER_TAG | (SMI_FIELD_OUT_OF_LINE | ((offset as i32) << SMI_SHIFT)) as u32 as u64
}

/// The probe of [`check_slots`]: an object with more fields than its in-object capacity, read
/// by named loads that run past the feedback threshold.
const SLOTS_SRC: &str = "\
var layoutWide = {};\n\
for (var layoutW = 0; layoutW < 12; layoutW++) layoutWide['w' + layoutW] = 100 + layoutW;\n\
function layoutWideGet(o) { return o.w11; }\n\
for (var layoutW = 0; layoutW < 20; layoutW++) layoutWideGet(layoutWide);\n";

/// The out-of-line exports ([`OBJECT_SLOTS`], [`SLOTS_DATA`], [`SMI_FIELD_OUT_OF_LINE`]; hand
/// facts until M7-P0) on a live object: the live facts of the warmed
/// load `o.w11` name an own field past the shape's capacity; the IC case of its shape is
/// [`field_out_of_line_handler`] of `SLOTS_DATA + 8 * (field - capacity)` (the handler kind and
/// the SLOTS data offset); the object's word at [`OBJECT_SLOTS`] is a cell whose word at that
/// offset is the field's value, and so is the word 8 below it for `w10`.
fn check_slots(session: &mut crate::harness::Session) -> Result<(), String> {
    use ep9_facts::{Access, SlotFacts};
    let r = session.run(SLOTS_SRC, "layout-slots.js");
    r.result.map_err(|e| format!("slots probe: {e}"))?;
    let o = session.function_value("layoutWide")?.to_bits();
    let f = session.function_value("layoutWideGet")?;
    let info = session.input_of(f)?.key;
    let script = session
        .script("layout-slots.js")
        .ok_or("no slots probe script")?;
    let id = *script
        .functions_named("layoutWideGet")
        .first()
        .ok_or("no layoutWideGet")?;
    let live = crate::facts::live(session, script)?;
    let facts = crate::facts::for_function(&live, script.unit.unit(), id);
    let Some((_, SlotFacts::LoadNamed(pf))) = facts.records.first() else {
        return Err(format!(
            "layoutWideGet: no LoadNamed record ({:?})",
            facts.records
        ));
    };
    let [case] = &pf.cases[..] else {
        return Err(format!("layoutWideGet: {} cases", pf.cases.len()));
    };
    let Access::OwnField { field } = case.access else {
        return Err(format!("layoutWideGet: access {:?}", case.access));
    };
    let cap = case.receiver.capacity;
    if field < cap {
        return Err(format!(
            "o.w11: field {field} is in object (capacity {cap}); the probe needs an out-of-line one"
        ));
    }
    let off = SLOTS_DATA + 8 * u32::from(field - cap);
    if !is_cell(o) {
        return Err("layoutWide is not a cell".into());
    }
    // SAFETY: `o` is a live cell held by a global of the session's isolate and `info` the
    // FunctionInfo of a live function; the isolate runs nothing while this reads. The SLOTS
    // word is checked a cell before it is read, and the probe's field is inside the cell (the
    // IC case for the object's shape names its offset).
    unsafe {
        if kind(o) != KIND_OBJECT {
            return Err(format!("layoutWide has kind {}", kind(o)));
        }
        let cell = feedback_cell(info)?.ok_or("layoutWideGet has no feedback vector")?;
        let (k, v) = feedback_slot(cell, 0);
        let h = ic_handler_for(k, v, shape_id(o)).ok_or("no IC case for layoutWide's shape")?;
        if h != field_out_of_line_handler(off) || h as i32 & SMI_KIND_MASK != SMI_FIELD_OUT_OF_LINE
        {
            return Err(format!(
                "IC handler {h:#x}, not FIELD_OUT_OF_LINE at SLOTS +{off} ({:#x})",
                field_out_of_line_handler(off)
            ));
        }
        let slots = word(o, OBJECT_SLOTS);
        if !is_cell(slots) {
            return Err(format!(
                "layoutWide +{OBJECT_SLOTS} is {slots:#x}, not a cell"
            ));
        }
        if word(slots, off) != int32_bits(111) || word(slots, off - 8) != int32_bits(110) {
            return Err(format!("SLOTS +{off} and +{} are not w11 and w10", off - 8));
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
        let note = check(&Config::default()).expect("layout");
        assert!(note.contains("FunctionInfo.feedback"), "{note}");
        assert!(note.contains("VM_LAYOUT 1"), "{note}");
        assert!(note.contains("Value encoding 1"), "{note}");
    }

    /// MC-w22-11: the pin names Value encoding 1, the version of `ep9_value::encoding`, and
    /// the read set's "Value encoding" owner prints the same number.
    #[test]
    fn value_encoding_is_pinned() {
        assert_eq!(PINNED[4], ("Value encoding", 1));
        assert_eq!(VM_VERSIONS[4], ep9_value::encoding::VERSION);
        let mut v = VM_VERSIONS;
        v[4] = 2;
        let e = check_versions(v).expect_err("refused");
        assert!(e.contains("Value encoding is 2"), "{e}");
        assert!(
            read_set_lines()
                .iter()
                .any(|l| l.starts_with("Value encoding 1: ")),
            "{:?}",
            read_set_lines()
        );
    }

    /// The pin refuses a VM with any other version.
    #[test]
    fn a_bumped_version_is_refused() {
        assert_eq!(check_versions(VM_VERSIONS), Ok(()));
        for i in 0..PINNED.len() {
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

    /// The script that pre-sets `layoutSiteB` (Codex M5-P3 r2 spk 1): an accessor that keeps
    /// the probe's store out and returns an empty array born PACKED_DOUBLE at a generalized
    /// `[]` site, whose elements word is `undefined` (no cell).
    const PRESET_SITE_B: &str = "\
function layoutMk() { return []; }\n\
var layoutE = null;\n\
for (var i = 0; i < 20; i++) layoutE = layoutMk();\n\
layoutE[0] = 1.5;\n\
var layoutEmpty = layoutMk();\n\
Object.defineProperty(globalThis, 'layoutSiteB', { get: function () { return layoutEmpty; }, set: function (v) {} });\n";

    /// A pre-set `layoutSiteB` accessor gives an `Err` from the probe, never a raw read of
    /// the non-cell elements word.
    #[test]
    fn a_preset_layout_site_b_accessor_is_refused() {
        let mut s = crate::harness::Session::new(&Config::default()).expect("session");
        s.run(PRESET_SITE_B, "preset.js").result.expect("preset");
        let e = s
            .function_value("layoutEmpty")
            .expect("layoutEmpty")
            .to_bits();
        // SAFETY: `e` is a live Array held by a global; the isolate runs nothing while this
        // reads its kind byte, its shape's kind bits and its elements word.
        let (k, ek, el) = unsafe { (kind(e), elements_kind(e), word(e, ARRAY_ELEMENTS)) };
        assert_eq!((k, ek), (KIND_ARRAY, ELEMENTS_KIND_PACKED_DOUBLE));
        assert!(!is_cell(el), "elements word {el:#x}");
        let r = check_in(&mut s);
        assert!(r.as_ref().is_err_and(|e| e.contains("born array")), "{r:?}");
    }

    #[test]
    fn handler_encoding() {
        assert_eq!(field_in_object_handler(24), NUMBER_TAG | (1 | (24 << 4)));
        assert_eq!(int32_of(int32_bits(-5)), Some(-5));
        assert_eq!(int32_of(double_bits(1.5)), None);
        assert!(!is_cell(int32_bits(0)) && !is_cell(HOLE) && !is_cell(0));
    }

    /// The VM's `layout.rs`: each public item and the owner its doc names first (`VM_LAYOUT.`,
    /// `FRAME.`, `OBJECT_LAYOUT (5).`, `IC_LAYOUT (MC-w16-4).`, `Value encoding.` ...).
    const LAYOUT_RS: &str = include_str!("../../../crates/ep9_vm/src/layout.rs");

    /// (name, owner) of each `pub const`, `pub const fn` and `pub fn` of [`LAYOUT_RS`] that has
    /// a doc block; the owner is the doc block's first words up to `.` or ` (`.
    fn layout_items() -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut doc: Option<String> = None;
        for l in LAYOUT_RS.lines() {
            let t = l.trim_start();
            if let Some(d) = t.strip_prefix("///") {
                doc.get_or_insert_with(|| d.trim().to_string());
                continue;
            }
            let item = ["pub const fn ", "pub const ", "pub fn "]
                .iter()
                .find_map(|p| l.strip_prefix(p));
            if let (Some(rest), Some(d)) = (item, doc.as_ref()) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let owner = d
                    .split(['.', '('])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                out.push((name, owner));
            }
            if !t.starts_with("#[") {
                doc = None;
            }
        }
        out
    }

    /// The names of `ep9_vm::layout` items the spike's sources use: `layout::NAME` (not
    /// `vm_layout::NAME`, which is this module) and the names of a `use ep9_vm::layout::{..}`.
    fn names_used(src: &str, exports: &std::collections::BTreeSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        };
        for (at, _) in src.match_indices("layout::") {
            let before = src[..at].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &src[at + "layout::".len()..];
            if let Some(list) = rest.strip_prefix('{') {
                let list = &list[..list.find('}').expect("a closed use list")];
                out.extend(list.split(',').map(|n| ident(n.trim())));
            } else {
                out.push(ident(rest));
            }
        }
        out.retain(|n| exports.contains(n));
        out
    }

    /// Every `.rs` file under `dir`, recursively.
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).expect("src dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// R17 read set (WI-SC9 (1)): [`READ_SET`] is exactly the `ep9_vm::layout` items the
    /// spike's sources read. A new read with no row (or a row nothing reads) fails here.
    #[test]
    fn read_set_is_what_spike_c_reads() {
        let exports: std::collections::BTreeSet<String> =
            layout_items().into_iter().map(|(n, _)| n).collect();
        assert!(exports.len() > 60, "{exports:?}");
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut used = std::collections::BTreeSet::new();
        for f in &files {
            let src = std::fs::read_to_string(f).expect("read");
            used.extend(names_used(&src, &exports));
        }
        let listed: std::collections::BTreeSet<String> =
            READ_SET.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(listed.len(), READ_SET.len(), "a name twice in READ_SET");
        assert_eq!(
            used.difference(&listed).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "read but not in READ_SET"
        );
        assert_eq!(
            listed.difference(&used).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "in READ_SET but not read"
        );
    }

    /// Each [`READ_SET`] owner is the owner `layout.rs` names for the item.
    #[test]
    fn read_set_names_the_owner_of_each_item() {
        let items = layout_items();
        for (name, owner) in READ_SET {
            let (_, doc) = items
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("{name} is not in layout.rs"));
            assert_eq!(doc, owner, "{name}");
        }
        let lines = read_set_lines();
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert!(
            lines[0].starts_with("OBJECT_LAYOUT 9: ") && lines[0].contains("SHAPE_CAGE_ALIGN"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("IC_LAYOUT 4: ") && lines[1].contains("VALUE_UNDEFINED"),
            "{}",
            lines[1]
        );
        assert_eq!(lines[5], "DEOPT_META 1: 1 exported: DUMP_BYTES");
        // No hand fact is left (v11in #2): every line lists exports only.
        assert!(!lines.iter().any(|l| l.contains("hand")), "{lines:?}");
        assert!(
            lines[4].starts_with("Value encoding 1: 2 exported: HOLE, NATIVE_WORD_OFFSET; 8 `ep9_value::encoding` constants: DOUBLE_ENCODE_OFFSET, "),
            "{}",
            lines[4]
        );
    }

    /// [`ENCODING_READS`] is exactly the `ep9_value::encoding` constants the spike's sources
    /// name (a path into `ep9_value::encoding`, or a name of a `use` list of it), `VERSION`
    /// aside. A new read with no row (or a row nothing reads) fails here.
    #[test]
    fn encoding_reads_are_what_spike_c_reads() {
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        };
        let mut used = std::collections::BTreeSet::new();
        for f in &files {
            let src = std::fs::read_to_string(f).expect("read");
            for (at, m) in src.match_indices("ep9_value::encoding::") {
                let rest = &src[at + m.len()..];
                if let Some(list) = rest.strip_prefix('{') {
                    let list = &list[..list.find('}').expect("a closed use list")];
                    used.extend(list.split(',').map(|n| ident(n.trim())));
                } else {
                    used.insert(ident(rest));
                }
            }
        }
        used.retain(|n| !n.is_empty() && n != "VERSION");
        let listed: std::collections::BTreeSet<String> =
            ENCODING_READS.iter().map(|n| n.to_string()).collect();
        assert_eq!(listed.len(), ENCODING_READS.len(), "a name twice");
        assert_eq!(used, listed);
    }

    /// The value after `prefix` in `src` (the first line that has it), up to `;` or `)`.
    fn value_after<'a>(src: &'a str, prefix: &str) -> &'a str {
        let l = src
            .lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .unwrap_or_else(|| panic!("no line {prefix}"));
        l[..l.find([';', ')']).expect("an end")].trim()
    }

    /// MC-w20-7, v11in #2, MC-w23-9: the 6 hand facts of month 5 (`HAND_FACTS` until M6-P0)
    /// are `ep9_vm::layout` items now, with the values the spike baked (so the switch changed
    /// no byte of compiled code) and the owners `layout.rs` names; each is in [`READ_SET`].
    /// The VM proves each on live objects (`tests::layout6`); [`check`] proves the shape rule
    /// and the two kind ids here too.
    #[test]
    fn the_former_hand_facts_are_exports() {
        assert_eq!(
            (SHAPE_CAGE_ALIGN, SHAPE_COUNTS, KIND_SHAPE, KIND_CONTEXT),
            (64 << 30, 48, 128, 66)
        );
        assert_eq!(
            ep9_vm::layout::VALUE_UNDEFINED,
            ep9_value::encoding::VALUE_UNDEFINED
        );
        assert_eq!(ep9_vm::layout::DUMP_BYTES, 256);
        let items = layout_items();
        for item in [
            ("SHAPE_CAGE_ALIGN", "OBJECT_LAYOUT"),
            ("SHAPE_COUNTS", "OBJECT_LAYOUT"),
            ("KIND_SHAPE", "OBJECT_LAYOUT"),
            ("KIND_CONTEXT", "OBJECT_LAYOUT"),
            ("VALUE_UNDEFINED", "IC_LAYOUT"),
            ("DUMP_BYTES", "DEOPT_META"),
        ] {
            assert!(
                items.iter().any(|(n, o)| (n.as_str(), o.as_str()) == item),
                "{item:?} in layout.rs"
            );
            assert!(READ_SET.contains(&item), "{item:?} in READ_SET");
        }
        assert_eq!(PINNED[5], ("DEOPT_META", 1));
        assert_eq!(VM_VERSIONS[5], ep9_vm::versions::DEOPT_META_VERSION);
    }

    /// The text of fn `name` in `src`: from `fn name(` to the next line that is `    }`.
    fn fn_text<'a>(src: &'a str, name: &str) -> &'a str {
        let at = src
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no fn {name}"));
        let body = &src[at..];
        &body[..body
            .find("\n    }\n")
            .unwrap_or_else(|| panic!("no end of fn {name}"))]
    }

    /// The unfilled IC key (`layout::VALUE_UNDEFINED`, IC_LAYOUT): the VM reads an entry
    /// whose key half is `undefined` as Uninitialized (`ic::state`), and a new FEEDBACK cell
    /// writes no key (the allocator pre-fills `undefined`); the emitter's shape guard compares
    /// the key with `VALUE_UNDEFINED` and takes its "unfilled" exit.
    #[test]
    fn the_unfilled_ic_key_is_the_vm_source() {
        const IC_RS: &str = include_str!("../../../crates/ep9_vm/src/ic.rs");
        const FRAME_RS: &str = include_str!("../../../crates/ep9_vm/src/frame.rs");
        const FEEDBACK_RS: &str = include_str!("../../../crates/ep9_vm/src/feedback.rs");
        const EMIT_RS: &str = include_str!("emit_a64/mod.rs");
        assert_eq!(
            ep9_vm::layout::VALUE_UNDEFINED,
            ep9_vm::Value::UNDEFINED.to_bits()
        );
        assert!(
            FRAME_RS.contains("pub const UNDEFINED: u64 = Value::UNDEFINED.to_bits();"),
            "frame::UNDEFINED"
        );
        let state = fn_text(IC_RS, "state");
        let first = state
            .find("if key == UNDEFINED {")
            .expect("ic::state tests the key with UNDEFINED");
        assert!(
            state[first..]
                .trim_start_matches("if key == UNDEFINED {")
                .trim_start()
                .starts_with("State::Uninitialized"),
            "{state}"
        );
        assert!(
            FEEDBACK_RS.contains("Keys stay `undefined` (the")
                && FEEDBACK_RS.contains("allocator's pre-fill)"),
            "feedback::make writes no key"
        );
        assert!(
            EMIT_RS.contains("layout::VALUE_UNDEFINED as u32")
                && EMIT_RS.contains("; cmp x17, undefined ; b.eq =>unfilled"),
            "the shape guard's unfilled compare"
        );
    }

    /// The shared exit's register dump (`layout::DUMP_BYTES`, DEOPT_META; MC-w22-6): the VM
    /// reads x0-x30 at the dump as `[u64; DUMP_REGS]` (word n = xn), and the emitter's
    /// `deopt_exit` makes `DUMP_BYTES` (the export: 16-aligned, room for the 31 words) below sp,
    /// stores each xn at `[sp, #8n]` and passes sp as the dump.
    #[test]
    fn the_register_dump_is_the_vm_source() {
        const DEOPT_RS: &str = include_str!("../../../crates/ep9_vm/src/deopt.rs");
        const NATIVE_RS: &str = include_str!("../../../crates/ep9_vm/src/native.rs");
        const EMIT_RS: &str = include_str!("emit_a64/mod.rs");
        let bytes = u64::from(ep9_vm::layout::DUMP_BYTES);
        let regs: u64 = value_after(DEOPT_RS, "pub(crate) const DUMP_REGS: usize =")
            .parse()
            .expect("DUMP_REGS");
        assert_eq!(regs, 31);
        assert!(
            NATIVE_RS.contains("[u64; crate::deopt::DUMP_REGS]>(dump as usize)"),
            "RuntimeTable::deopt reads the dump as [u64; DUMP_REGS]"
        );
        assert_eq!(
            value_after(EMIT_RS, "const DUMP_BYTES: u32 ="),
            "layout::DUMP_BYTES",
            "the emitter's dump is the export"
        );
        assert!(bytes % 16 == 0 && 8 * regs <= bytes && bytes < 8 * regs + 16);
        let exit = fn_text(EMIT_RS, "deopt_exit");
        assert!(exit.contains("; sub sp, sp, DUMP_BYTES"), "{exit}");
        assert!(exit.contains("; add x3, sp, #0"), "the dump is sp: {exit}");
        // Each store of the dump: `str xA, [sp, #o]` or `stp xA, xB, [sp(, #o)]`.
        let mut at = std::collections::BTreeMap::new();
        for l in exit.lines() {
            let Some(rest) = l
                .trim()
                .strip_prefix("; stp ")
                .or_else(|| l.trim().strip_prefix("; str "))
            else {
                continue;
            };
            let (list, addr) = rest.split_once(", [sp").expect("a store below sp");
            let off: u64 = match addr.strip_suffix(']').expect("a closed address") {
                "" => 0,
                o => o
                    .strip_prefix(", #")
                    .expect("an offset")
                    .parse()
                    .expect("a number"),
            };
            for (i, r) in list.split(", ").enumerate() {
                let n: u64 = r.trim_start_matches('x').parse().expect("an x register");
                assert!(at.insert(n, off + 8 * i as u64).is_none(), "x{n} twice");
            }
        }
        assert_eq!(at.len() as u64, regs, "{at:?}");
        for (n, off) in at {
            assert_eq!(off, 8 * n, "x{n}");
        }
    }
}
