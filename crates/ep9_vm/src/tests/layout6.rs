//! The OBJECT_LAYOUT 6, IC_LAYOUT 4 and VM_LAYOUT 1 tests (M6-P0 VM6, plan v11 §2.4 step 8):
//! the version pins, kind 84 registered, the FUSE cell size and the `Root::RealmFuses` index
//! against live objects, the (IC_LAYOUT_VERSION, keyed kinds-set mask) pair, the shape flags
//! probe at bit 30, and each hand-fact export against the live value. The module is declared in
//! VM6 step 0 with the other month-6 test modules; VM6 step 8 fills it.

use std::hint::black_box;

use ep9_bytecode::{FunctionId, VerifiedUnit};
use ep9_heap::{CellKind, CellSize, KindError};

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::builtins::{self, IntrinsicId};
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::object::{cell, realmf};
use crate::runtime::{REALM_ROOT_COUNT, ROOT_COUNT, Root, Runtime};
use crate::{Dispatch, IsolateConfig, kinds, layout, object, props, shape, versions};

fn no_trace(_: ep9_heap::CellPtr, _: &mut ep9_heap::Tracer<'_>) {}

fn dummy_kind(name: &'static str) -> CellKind {
    CellKind {
        name,
        size: CellSize::Fixed(16),
        trace: no_trace,
    }
}

/// The word at byte `off` of the cell value `c`.
///
/// # Safety
/// `c` is a live cell of an isolate that runs nothing while this reads, at least `off + 8`
/// bytes long.
unsafe fn word_at(c: u64, off: usize) -> u64 {
    // SAFETY: caller contract.
    unsafe { ((c as usize + off) as *const u64).read() }
}

/// The kind byte of the cell value `c` (as [`word_at`]).
unsafe fn kind_byte(c: u64) -> u8 {
    // SAFETY: caller contract.
    unsafe { ((c as usize + layout::KIND_BYTE as usize) as *const u8).read() }
}

/// The 4 OBJECT_LAYOUT hand-fact exports of M6-P0 step 3 (were Spike C `HAND_FACTS`) on live
/// cells: the shape rule with [`layout::SHAPE_CAGE_ALIGN`], the shape's `counts` word at
/// [`layout::SHAPE_COUNTS`] (its fields equal the shape accessors), and the SHAPE and CONTEXT
/// kind ids.
#[test]
fn the_object_layout_hand_fact_exports_hold_on_live_cells() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    assert!(
        rt.heap
            .cage_base()
            .is_multiple_of(layout::SHAPE_CAGE_ALIGN as usize)
    );
    let o = object::new_object(rt, rt.root(crate::runtime::Root::ObjectPrototype));
    let a = object::new_array(rt, 2);
    object::array_set(rt, a, 0, crate::frame::int32_bits(5));
    let c = object::new_context(rt, UNDEFINED, core::ptr::null(), 2);
    for v in [o, a] {
        let s = shape::shape_of(v);
        // SAFETY: `v`, its shape and `c` are live cells of `iso`, which runs nothing here.
        unsafe {
            let id = u64::from((word_at(v, 0) as u32) >> (8 * layout::SHAPE_ID_BYTE));
            assert_eq!((v & !(layout::SHAPE_CAGE_ALIGN - 1)) | id, s);
            assert_eq!(kind_byte(s), layout::KIND_SHAPE);
            let counts = int32_of(word_at(s, layout::SHAPE_COUNTS as usize));
            let bits = counts as u32;
            assert_eq!(
                (bits >> layout::SHAPE_ELEMENTS_KIND_SHIFT) as u8,
                shape::elements_kind(s)
            );
            assert_eq!(
                (bits >> 20) as u8,
                shape::object_kind(s),
                "counts bits 20-27"
            );
            assert_eq!((bits >> 12) as u8, shape::capacity(s), "counts bits 12-19");
            assert_eq!((bits & 0xFFF) as usize, shape::count(s), "counts bits 0-11");
        }
    }
    // SAFETY: as above.
    unsafe {
        assert_eq!(kind_byte(c), layout::KIND_CONTEXT);
        assert_eq!(kind_byte(c), kinds::CONTEXT.get());
    }
    assert_eq!(
        (
            layout::SHAPE_CAGE_ALIGN,
            layout::SHAPE_COUNTS,
            layout::KIND_SHAPE,
            layout::KIND_CONTEXT
        ),
        (64 << 30, 48, 128, 66),
        "the values Spike C baked as hand facts (OBJECT_LAYOUT 5)"
    );
}

/// `f(a)`: `if (a) return undefined; return g` (its global-load slot stays unfilled when
/// `a` is true).
fn unfilled_unit() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 1, 0, 0, 0));
    let a0 = f.b.parameter(0);
    let skip = f.b.new_label();
    f.b.ldar(a0).jump_if_true(skip);
    f.lda_global("g");
    f.b.return_();
    f.b.bind(skip);
    f.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish()])
}

/// A script: `g = 1` if `define_g`, then `f(a)`.
fn call_unit(a: bool, define_g: bool) -> VerifiedUnit {
    let mut s = Asm::script(0);
    if define_g {
        s.b.lda_smi(1);
        s.sta_global("g");
    }
    let (f, a0) = (s.f, s.a.get(0));
    if a {
        s.b.lda_true();
    } else {
        s.b.lda_false();
    }
    s.b.star(a0);
    s.lda_global("f");
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// The IC_LAYOUT and DEOPT_META hand-fact exports of M6-P0 step 3: a feedback slot that no
/// site filled has the key [`layout::VALUE_UNDEFINED`] and reads Uninitialized, and a filled
/// one does not; [`layout::DUMP_BYTES`] holds the `deopt::DUMP_REGS` words, 16-aligned.
#[test]
fn the_ic_and_deopt_hand_fact_exports_hold() {
    let c = IsolateConfig {
        feedback_threshold: 0,
        ..config(Dispatch::DEFAULT, false)
    };
    let mut iso = isolate(c);
    let mut run =
        |u: VerifiedUnit| iso.with_cx_checkpoint(|cx| cx.run_script(u, "t.js").map(|_| ()));
    run(unfilled_unit()).unwrap();
    run(call_unit(true, false)).unwrap();
    let f = iso.global_raw("f").unwrap();
    let fi = object::function_info(f.to_bits());
    let v = crate::feedback::vector(fi).expect("a vector (threshold 0)");
    // `Asm::new`'s `load` slot (the 4th, index 3) is the global load's slot.
    assert_eq!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    assert_eq!(crate::ic::state(v, 3), crate::ic::State::Uninitialized);
    // Control: the load of a missing `g` throws and records no case; once `g` exists, the
    // load fills the key, which is then not the unfilled value.
    let mut run =
        |u: VerifiedUnit| iso.with_cx_checkpoint(|cx| cx.run_script(u, "t.js").map(|_| ()));
    assert!(run(call_unit(false, false)).is_err());
    assert_eq!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    run(call_unit(false, true)).unwrap();
    assert_ne!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    assert_eq!(crate::ic::state(v, 3), crate::ic::State::Monomorphic);
    assert_eq!(UNDEFINED, ep9_value::encoding::VALUE_UNDEFINED);

    assert_eq!(
        layout::DUMP_BYTES,
        256,
        "the value Spike C baked (DEOPT_META 0)"
    );
    assert!(layout::DUMP_BYTES as usize >= 8 * crate::deopt::DUMP_REGS);
    assert!(layout::DUMP_BYTES.is_multiple_of(16));
    assert_eq!(crate::deopt::DUMP_REGS, 31, "x0-x30");
}

// ---- versions (M6-P0 VM6 step 8) -------------------------------------------------------------

/// The month-6 versions (V11-D3, plan v11 rule 2): OBJECT_LAYOUT 5 -> 6 and IC_LAYOUT 3 -> 4
/// move once, in M6-P0; VM_LAYOUT 1 (a RuntimeTable append moves none, C-18), FRAME 2,
/// DEOPT_META 0 (V11-D12), the builtin ABI 1 and the bytecode ABI 5 stay.
#[test]
fn month_6_versions() {
    assert_eq!(
        (
            black_box(versions::OBJECT_LAYOUT_VERSION),
            versions::IC_LAYOUT_VERSION,
            versions::VM_LAYOUT_VERSION,
            versions::FRAME_VERSION,
            versions::DEOPT_META_VERSION
        ),
        (6, 4, 1, 2, 0)
    );
    assert_eq!(
        (versions::BUILTIN_ABI_VERSION, ep9_bytecode::ABI_VERSION),
        (1, 5)
    );
}

// ---- OBJECT_LAYOUT 6: kind 84, the FUSE cell, Root::RealmFuses ------------------------------

/// Masks a cell address so the test's frame never holds it (the conservative scan, H10).
const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Overwrites 256 KiB of stack below the caller (plan v10 rule 35), so stale words of returned
/// frames hold no cell for the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Makes an object, stores it as the value of entry 0 of the PROPS_INDEXED cell `c` and
/// returns a weak handle to it and its masked address; out of line, so the object stays in no
/// frame of the caller.
#[inline(never)]
fn hold_in_entry_0(rt: &Runtime, c: ep9_heap::CellPtr) -> (ep9_heap::WeakHandle, usize) {
    use crate::props_index::pif;
    let held = object::new_object(rt, rt.root(Root::ObjectPrototype));
    object::wr(&rt.heap, c, pif::ENTRIES + 8, held);
    let h = rt.heap.new_weak(crate::runtime::val(held));
    (h, black_box(held as usize) ^ MASK)
}

/// Kind 84 PROPS_INDEXED is registered on every isolate (`props::register_kinds`), with the
/// layout the export names: a live cell of capacity 3 is `32 + 24 * 3` bytes by its size
/// function, and its trace function keeps alive an object that only one of its entries holds:
/// after a scrub and a full GC whose conservative scan did not find that object, a weak handle
/// still reads it, and the heap verifier passes (plain and under GC stress 1).
#[test]
fn kind_84_is_registered_with_its_size_and_trace() {
    use crate::props_index::pif;
    assert_eq!(kinds::PROPS_INDEXED.get(), 84);
    assert_eq!(layout::KIND_PROPS_INDEXED, 84);
    assert_eq!(
        (
            layout::PROPS_INDEXED_CAPACITY,
            layout::PROPS_INDEXED_COUNT,
            layout::PROPS_INDEXED_INDEX,
            layout::PROPS_INDEXED_ENTRIES,
            layout::PROPS_INDEXED_ENTRY_SIZE
        ),
        (8, 16, 24, 32, 24)
    );
    for stress in [false, true] {
        let iso = isolate(config(Dispatch::DEFAULT, stress));
        let rt = &*iso.rt;
        match rt
            .heap
            .register_kind(kinds::PROPS_INDEXED, dummy_kind("test"))
        {
            Err(KindError::Taken(k)) => assert_eq!(k, kinds::PROPS_INDEXED),
            r => panic!("kind 84 is not registered: {r:?}"),
        }
        const CAP: usize = 3;
        let key = crate::atoms::intern_str(rt, "layout6Key");
        let c = rt.alloc(kinds::PROPS_INDEXED, pif::ENTRIES + pif::ENTRY * CAP);
        object::init(c, pif::CAPACITY, int32_bits(CAP as i32));
        object::init(c, pif::COUNT, int32_bits(1));
        object::init(c, pif::INDEX, UNDEFINED);
        for i in 0..CAP {
            let e = pif::ENTRIES + pif::ENTRY * i;
            object::init(c, e, if i == 0 { key } else { UNDEFINED });
            object::init(c, e + 8, UNDEFINED);
            object::init(c, e + 16, int32_bits(0));
        }
        let root = rt.heap.new_global(c.to_value());
        // SAFETY: `c` is a live cell of this heap.
        assert_eq!(unsafe { rt.heap.cell_size(c) }, 32 + 24 * CAP);
        let (weak, masked) = hold_in_entry_0(rt, c);
        scrub_stack();
        rt.heap.collect();
        let held = masked ^ MASK;
        assert!(
            !rt.heap
                .last_conservative_roots()
                .iter()
                .any(|r| r.addr() == held),
            "the conservative scan kept the entry's object: the check proves nothing"
        );
        let v = rt
            .heap
            .weak(weak)
            .expect("the PROPS_INDEXED trace keeps entry 0's value");
        let cv = rt.heap.global(root).to_bits();
        // SAFETY: `cv` is held by the global handle, so it is live; the isolate runs nothing.
        unsafe {
            assert_eq!(kind_byte(cv), layout::KIND_PROPS_INDEXED);
            assert_eq!(word_at(cv, pif::ENTRIES), key);
            assert_eq!(word_at(cv, pif::ENTRIES + 8), v.to_bits());
            assert_eq!(int32_of(word_at(cv, pif::CAPACITY)), CAP as i32);
        }
        assert!(object::is_object(v.to_bits()));
        iso.verify_heap().unwrap();
        rt.heap.drop_weak(weak);
        rt.heap.drop_global(root);
    }
}

/// The FUSE 83 layout of OBJECT_LAYOUT 6 (built by M6-P2 FUSE-PI): one u32 per static fuse
/// from +8, and `Root::RealmFuses`, the realm root before `Root::RealmKey` that mirrors the
/// intrinsic `IntrinsicId::RealmFuses`. Each realm's block is made at realm init with the
/// exported kind and size: the entries read at the exported offsets (only static fuse 6,
/// "no-elements prototypes", is wired and intact), a second realm has its own block, and a
/// realm switch carries the root.
#[test]
fn the_fuse_layout_and_the_realm_fuses_root() {
    let n = crate::fuse::STATIC.len();
    assert_eq!(n, 11, "fuse::STATIC");
    assert_eq!(
        (
            layout::KIND_FUSE,
            layout::FUSE_ENTRIES,
            layout::FUSE_ENTRY_SIZE,
            layout::FUSE_INTACT
        ),
        (83, 8, 4, 1)
    );
    assert_eq!(layout::KIND_FUSE, kinds::FUSE.get());
    assert_eq!(layout::FUSE_SIZE, 56, "8 + 4 x 11, rounded up to 8");
    // The root: the last realm root, before the isolate-wide RealmKey (V11-D21, MC-w22-17).
    assert_eq!(Root::RealmFuses as usize + 1, Root::RealmKey as usize);
    assert_eq!(Root::RealmFuses as usize, REALM_ROOT_COUNT - 1);
    assert_eq!(REALM_ROOT_COUNT + 1, ROOT_COUNT);
    assert_eq!(IntrinsicId::RealmFuses as usize, 72);
    assert_eq!(IntrinsicId::ALL[72], IntrinsicId::RealmFuses);
    assert_eq!(IntrinsicId::RealmFuses.root(), Some(Root::RealmFuses));
    assert!(!IntrinsicId::RealmFuses.is_core());
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let first = iso.cx.realm;
        let rt = &*iso.rt;
        // The realm's block: in the root, the intrinsic and the record; kind 83 is taken.
        let cv = rt.root(Root::RealmFuses);
        assert_eq!(builtins::intrinsic(rt, IntrinsicId::RealmFuses), cv);
        let list = object::record_get(first, realmf::INTRINSICS);
        assert_eq!(object::list_len(list), IntrinsicId::ALL.len());
        assert_eq!(object::list_get(list, 72), cv);
        assert_eq!(
            rt.heap
                .register_kind(kinds::FUSE, dummy_kind("fuse-block (test)")),
            Err(KindError::Taken(kinds::FUSE)),
            "FUSE-PI registers kind 83"
        );
        let c = cell(cv);
        // SAFETY: `c` is a live cell of this heap (the root holds it).
        assert_eq!(unsafe { rt.heap.cell_size(c) }, 56);
        let read = |c: u64, i: usize| {
            let at =
                c as usize + (layout::FUSE_ENTRIES + layout::FUSE_ENTRY_SIZE * i as u32) as usize;
            // SAFETY: `c` is a live FUSE cell, entry `i` < n.
            unsafe { (at as *const u32).read() }
        };
        let r = iso.with_cx(|cx| {
            let g = cx.create_realm()?;
            cx.with_realm(g, |cx| {
                let rt: &Runtime = cx.inner().rt();
                let own = rt.root(Root::RealmFuses);
                assert!(own != cv && own != UNDEFINED, "a realm's own block");
                // SAFETY: the root holds the second realm's live block.
                assert_eq!(unsafe { kind_byte(own) }, layout::KIND_FUSE);
                rt.heap.collect();
                Ok(())
            })?;
            let rt: &Runtime = cx.inner().rt();
            assert_eq!(
                rt.root(Root::RealmFuses),
                cv,
                "the switch back loads the root"
            );
            Ok(())
        });
        assert_eq!(r, Ok(()));
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        // SAFETY: the FUSE cell is held by the root (and this frame), so it is live.
        unsafe {
            assert_eq!(kind_byte(cv), layout::KIND_FUSE);
        }
        let words: Vec<u32> = (0..n).map(|i| read(cv, i)).collect();
        assert_eq!(words, [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
    });
}

// ---- IC_LAYOUT 4: the kinds-set pair and the ran bit -----------------------------------------

/// IC_LAYOUT 4 (V11-D4): the keyed kinds set is bits 10-25, one bit per kind value 0-15, and
/// the pair (IC_LAYOUT_VERSION, mask) is pinned (as in `tests::elements`); the LoadKeyed "ran"
/// bit 26 is outside the set and bits 0-9; bits 27-30 are reserved; the new IC_HANDLER kinds
/// are 9-11, after ALLOC_SITE 8 and inside the int32 at +8.
#[test]
fn the_kinds_set_pair_and_the_ran_bit() {
    use crate::elements::{self, kind as ek};
    use crate::ic::hk;
    use crate::profile::keyed;
    assert_eq!(
        (versions::IC_LAYOUT_VERSION, elements::KINDS_SET_MASK),
        (4, 0xFFFF << 10)
    );
    assert_eq!(layout::KEYED_KINDS_SET_SHIFT, 10);
    for k in 0..=ek::MAX {
        let bit = elements::kinds_set_bit(k);
        assert_eq!(bit, 1 << (10 + u32::from(k)), "kind {k}");
        assert_ne!(bit & elements::KINDS_SET_MASK, 0, "kind {k}");
    }
    let low = keyed::INDEX
        | keyed::NAME
        | keyed::SYMBOL
        | keyed::OTHER_KEY
        | keyed::ARRAY
        | keyed::OBJECT
        | keyed::STRING
        | keyed::PRIMITIVE
        | keyed::OUT_OF_BOUNDS
        | keyed::HOLE;
    assert_eq!(low, 0x3FF, "bits 0-9");
    assert_eq!(keyed::RAN, 1 << 26);
    assert_eq!(keyed::RAN & (low | elements::KINDS_SET_MASK), 0);
    const RESERVED: u32 = 0xF << 27;
    assert_eq!(RESERVED & (low | elements::KINDS_SET_MASK | keyed::RAN), 0);
    assert_eq!(
        low | elements::KINDS_SET_MASK | keyed::RAN | RESERVED,
        0x7FFF_FFFF,
        "bits 0-30, bit 31 unused"
    );
    assert_eq!(
        (hk::ALLOC_SITE, hk::GETTER, hk::SETTER, hk::PROTO_DICT_LOAD),
        (8, 9, 10, 11)
    );
}

/// The "ran" bit 26 is free on live slots (V11-D4: "VM6 proves it free"): a LoadKeyed and a
/// StoreKeyed site that saw arrays of 5 kinds, DICTIONARY included (bit 16, the top bit of the
/// IC_LAYOUT 3 set), hold their kinds in bits 10-25 and nothing in bits 26-31.
#[test]
fn the_ran_bit_is_free_on_recorded_keyed_slots() {
    use ep9_bytecode::FeedbackSlotKind;

    use crate::elements::{self, kind as ek};
    use crate::feedback;
    use crate::profile::keyed;
    let mut s = Asm::script(0);
    let get = s.b.feedback_slot(FeedbackSlotKind::LoadKeyed);
    let set = s.set_keyed;
    let k = s.b.new_register();
    let mut arrays = Vec::new();
    for (at, v) in [(0, 0), (0, 1), (0, 2), (1, 0), (5000, 0)] {
        let r = s.b.new_register();
        let lit = s.lit;
        s.b.create_empty_array_literal(lit).star(r);
        s.b.lda_smi(at).star(k);
        match v {
            0 => {
                s.b.lda_smi(5);
            }
            1 => {
                s.lda_num(1.5);
            }
            _ => {
                s.lda_str("x");
            }
        }
        s.b.set_keyed_property(r, k, set);
        arrays.push(r);
    }
    for r in arrays {
        s.b.lda_zero().get_keyed_property(r, get);
    }
    s.b.lda_undefined().return_();
    let u = unit("", vec![s.finish()]);
    let mut iso = super::profile::isolate(IsolateConfig {
        feedback_threshold: 0,
        ..config(Dispatch::DEFAULT, false)
    });
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "ran.js").map(|_| ()));
    assert!(r.is_ok(), "{r:?}");
    let units = iso.rt.units.borrow();
    let fi = &units.last().unwrap().infos[0];
    let v = feedback::vector(fi).unwrap();
    for slot in [get, set] {
        let bits = feedback::value(v, slot.index() as u32) as u32;
        assert_eq!(bits & !0x03FF_FFFF, 0, "bits 26-31 of {bits:#x}");
        assert_eq!(bits & keyed::RAN, 0);
        assert_ne!(bits & keyed::INDEX, 0, "an index key: {bits:#x}");
    }
    let g = feedback::value(v, get.index() as u32) as u32;
    assert_ne!(
        g & elements::kinds_set_bit(ek::DICTIONARY),
        0,
        "DICTIONARY, bit 16: {g:#x}"
    );
}

// ---- the shape flags probe -------------------------------------------------------------------

/// H17 "shape flag bits (append-only)" (V11-D7, V11-A9; MODES-CHECK-w23 MC-w23-18): bits 4-30
/// of a shape's `flags` word are append room and bit 31 is never used. The probe is bit 30, the
/// top of the room (never the next free bit 4): no flag constant names it, and a shape with it
/// set reads and behaves as the same shape (its own flags, a property read and add, a GC and
/// the heap verifier); a child shape made from it keeps the bit, as `shape::add` copies flags.
#[test]
fn h17_shape_flag_bit_probe_30() {
    use crate::shape::flag;
    const PROBE: i32 = 1 << 30;
    /// The `flags` word of a shape cell (`shape.rs` `shf::FLAGS`, a private module; the test
    /// reads it back through `shape::flags`).
    const FLAGS: usize = 56;
    let built = flag::EXTENSIBLE | flag::DICTIONARY | flag::PROTOTYPE | flag::UNIQUE;
    assert_eq!(built, 0xF, "bits 0-3 as built");
    assert_eq!(flag::SLACK_MASK, 0xF0, "bits 4-7, unused");
    const _: () = assert!(
        PROBE > 0,
        "bit 30 keeps the int32 positive; bit 31 is never used"
    );
    for stress in [false, true] {
        let iso = isolate(config(Dispatch::DEFAULT, stress));
        let rt = &*iso.rt;
        let o = object::new_object(rt, rt.root(Root::ObjectPrototype));
        let a = crate::atoms::intern_str(rt, "layout6A");
        props::own_add(rt, o, a, int32_bits(7), object::attr::DEFAULT);
        let s = shape::shape_of(o);
        let before = shape::flags(s);
        assert_eq!(before & !0xF, 0, "no flag above bit 3 today");
        object::init(cell(s), FLAGS, int32_bits(before | PROBE));
        assert_eq!(shape::flags(s), before | PROBE, "shf::FLAGS is +56");
        assert_eq!(shape::is_extensible(s), before & flag::EXTENSIBLE != 0);
        assert_eq!(shape::is_dictionary(s), before & flag::DICTIONARY != 0);
        assert_eq!(
            props::own_get(o, a),
            Some((int32_bits(7), object::attr::DEFAULT))
        );
        let b = crate::atoms::intern_str(rt, "layout6B");
        props::own_add(rt, o, b, int32_bits(8), object::attr::DEFAULT);
        let t = shape::shape_of(o);
        assert_ne!(t, s);
        assert_eq!(shape::flags(t) & PROBE, PROBE, "a child keeps the bit");
        assert_eq!(
            props::own_get(o, a),
            Some((int32_bits(7), object::attr::DEFAULT))
        );
        assert_eq!(
            props::own_get(o, b),
            Some((int32_bits(8), object::attr::DEFAULT))
        );
        rt.heap.collect();
        iso.verify_heap().unwrap();
        assert_eq!(shape::flags(shape::shape_of(o)) & PROBE, PROBE);
        black_box(o);
    }
}

/// RuntimeTable entry 11 `load_global` (M6-P0 VM6 step 2, V11-D43): a Rust function stands in
/// for native code. It is installed as the native code of a bytecode function and calls the
/// entry for that function's global load, so the entry runs with a real native frame.
/// Each test name starts with `native_`: CI runs the spike-native tests with the name filter
/// `native` (`.github/workflows/ci.yml`), and these tests build only with spike-native.
#[cfg(feature = "spike-native")]
mod load_global {
    use core::cell::Cell;

    use ep9_bytecode::{
        FeedbackSlotKind, FunctionId, Opcode, RegisterList, RuntimeId, ScopeKind, VariableMode,
        VerifiedUnit,
    };
    use ep9_value::Value;

    use crate::dispatch::DispatchKind;
    use crate::native::{NativeCx, RuntimeTable};
    use crate::tests::{Asm, config, isolate, meta, on_big_stack, unit_with_scopes};
    use crate::{Dispatch, Exception, Isolate, IsolateConfig};

    thread_local! {
        /// The bytecode offset the stand-in passes to the entry. It is per thread: each test
        /// installs and calls on its own thread, and the entry runs on the thread that calls.
        /// A shared value let a parallel test change the offset between install and call.
        static OFF: Cell<u32> = const { Cell::new(0) };
    }

    /// The stand-in for native code: the entry's result for the load at [`OFF`].
    #[cfg(target_arch = "x86_64")]
    unsafe extern "sysv64" fn entry(cx: *mut NativeCx, fp: *mut u64) -> u64 {
        // SAFETY: called as native code, with the Cx and the live frame of its function.
        unsafe { (RuntimeTable::process().load_global)(cx, fp, OFF.with(Cell::get)) }
    }
    /// The stand-in for native code: the entry's result for the load at [`OFF`].
    #[cfg(not(target_arch = "x86_64"))]
    unsafe extern "C" fn entry(cx: *mut NativeCx, fp: *mut u64) -> u64 {
        // SAFETY: called as native code, with the Cx and the live frame of its function.
        unsafe { (RuntimeTable::process().load_global)(cx, fp, OFF.with(Cell::get)) }
    }

    /// `x = 7`; the functions below; `let tdz` declared, then a throw before it is set, so
    /// `tdz` stays in its TDZ.
    /// lg(a): `if (a) return undefined; return x` (the load after a jump, at offset > 0);
    /// lt(): `return typeof nope` (the load part); lm(): `return nope`; lz(): `return tdz`;
    /// nl(): `return undefined` (no global load at offset 0).
    fn defs() -> VerifiedUnit {
        let mut lg = Asm::new(meta("lg", 1, 0, 0, 0));
        let a0 = lg.b.parameter(0);
        let skip = lg.b.new_label();
        lg.b.ldar(a0).jump_if_true(skip);
        lg.lda_global("x");
        lg.b.return_();
        lg.b.bind(skip);
        lg.b.lda_undefined().return_();
        let mut lt = Asm::new(meta("lt", 0, 0, 0, 0));
        let k = lt.b.string_constant("nope");
        let typeof_slot = lt.b.feedback_slot(FeedbackSlotKind::LoadGlobalInsideTypeof);
        lt.b.lda_global_inside_typeof(k, typeof_slot).return_();
        let mut lm = Asm::new(meta("lm", 0, 0, 0, 0));
        lm.lda_global("nope");
        lm.b.return_();
        let mut lz = Asm::new(meta("lz", 0, 0, 0, 0));
        lz.lda_global("tdz");
        lz.b.return_();
        let mut nl = Asm::new(meta("nl", 0, 0, 0, 0));
        nl.b.lda_undefined().return_();
        let fns = vec![("lg", lg), ("lt", lt), ("lm", lm), ("lz", lz), ("nl", nl)];
        let mut s = Asm::script(0);
        s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
        s.b.lda_smi(7);
        s.sta_global("x");
        for (i, (name, _)) in fns.iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        s.b.lda_smi(1).throw();
        let mut all = vec![s.finish()];
        all.extend(fns.into_iter().map(|(_, a)| a.finish()));
        unit_with_scopes(
            "",
            vec![(ScopeKind::Script, None, vec![("tdz", VariableMode::Let)])],
            all,
        )
    }

    /// An isolate that ran `defs()` (it throws 1 at its end, after the definitions).
    fn defs_isolate(config: IsolateConfig) -> Isolate {
        let mut iso = isolate(config);
        let e = iso
            .with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
            .unwrap_err();
        assert_eq!(e.message, "1");
        iso
    }

    /// The offset of the first `op` in the global function `name`.
    fn offset_of(iso: &Isolate, name: &str, op: Opcode) -> u32 {
        let t = crate::native_target(iso, iso.global_raw(name).unwrap()).unwrap();
        ep9_bytecode::Instructions::new(t.bytecode)
            .map(|i| i.unwrap())
            .find(|i| i.opcode == op)
            .unwrap_or_else(|| panic!("no {op:?} in {name}"))
            .offset
    }

    /// Sets the offset that [`entry`] passes to the entry.
    fn set_off(off: u32) {
        OFF.with(|o| o.set(off));
    }

    /// Installs [`entry`] as the native code of `name`, with the entry offset `off`.
    fn install(iso: &Isolate, name: &str, off: u32) -> Value {
        set_off(off);
        let f = iso.global_raw(name).unwrap();
        let start = entry as *const () as usize;
        // SAFETY: the range starts at `entry`, a function with the native entry ABI that lives
        // as long as the process.
        unsafe { crate::install_native(iso, f, DispatchKind::Baseline, start..start + 4) }.unwrap();
        f
    }

    /// The result bits of a call of `f` from Rust (the native entry of `f` if it has one).
    fn call(iso: &mut Isolate, f: Value, args: &[Value]) -> Result<u64, Exception> {
        crate::call_from_rust(iso, f, Value::UNDEFINED, args).map(Value::to_bits)
    }

    /// The value of a global data property, `undefined` for a missing name in `typeof`, the
    /// T0 ReferenceErrors for a missing name and a TDZ binding, and an internal error for an
    /// offset that holds no global load. Each run in a new isolate (one install each).
    #[test]
    fn native_load_global_runs_the_t0_lookup() {
        for &d in Dispatch::ALL {
            on_big_stack(move || {
                let mut iso = defs_isolate(config(d, false));
                let off = offset_of(&iso, "lg", Opcode::LdaGlobal);
                assert!(off > 0, "the load follows a jump");
                let f = install(&iso, "lg", off);
                assert_eq!(
                    call(&mut iso, f, &[Value::FALSE]).unwrap(),
                    Value::int32(7).to_bits()
                );

                let mut iso = defs_isolate(config(d, true));
                let f = install(&iso, "lt", 0);
                assert_eq!(call(&mut iso, f, &[]).unwrap(), Value::UNDEFINED.to_bits());

                let mut iso = defs_isolate(config(d, true));
                let f = install(&iso, "lm", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert_eq!(e.constructor.as_deref(), Some("ReferenceError"));
                assert_eq!(e.message, "ReferenceError: nope is not defined");

                let mut iso = defs_isolate(config(d, false));
                let f = install(&iso, "lz", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert_eq!(
                    e.message,
                    "ReferenceError: Cannot access 'tdz' before initialization"
                );

                let mut iso = defs_isolate(config(d, false));
                let f = install(&iso, "nl", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert!(
                    e.message.contains("no global load at offset 0"),
                    "{}",
                    e.message
                );
            });
        }
    }

    /// MC-w15-6: the entry records no feedback. The vector of `lg` exists (threshold 0, a T0
    /// call that skips the load); the native load leaves the slot of the load Uninitialized.
    /// Control: the T0 load of the same function records a case there.
    #[test]
    fn native_load_global_records_no_feedback() {
        on_big_stack(|| {
            let c = IsolateConfig {
                feedback_threshold: 0,
                ..config(Dispatch::Match, false)
            };
            let mut iso = defs_isolate(c);
            let f = iso.global_raw("lg").unwrap();
            assert_eq!(
                call(&mut iso, f, &[Value::TRUE]).unwrap(),
                Value::UNDEFINED.to_bits()
            );
            let state = || {
                let fi = crate::object::function_info(f.to_bits());
                let v = crate::feedback::vector(fi).expect("a vector (threshold 0)");
                // `lg` has one global-load slot, `Asm::new`'s `load` slot (the 4th, index 3).
                crate::ic::state(v, 3)
            };
            assert_eq!(state(), crate::ic::State::Uninitialized);
            let off = offset_of(&iso, "lg", Opcode::LdaGlobal);
            install(&iso, "lg", off);
            for _ in 0..3 {
                assert_eq!(
                    call(&mut iso, f, &[Value::FALSE]).unwrap(),
                    Value::int32(7).to_bits()
                );
            }
            assert_eq!(state(), crate::ic::State::Uninitialized);
            crate::uninstall_native(&iso, f).unwrap();
            assert_eq!(
                call(&mut iso, f, &[Value::FALSE]).unwrap(),
                Value::int32(7).to_bits()
            );
            assert_eq!(state(), crate::ic::State::Monomorphic);
        });
    }

    /// The CI step that runs the spike-native tests filters them by `native`, so the path of
    /// each test here must hold that word, or CI never runs it. This reads the source of this
    /// module: each `#[test]` function name must start with `native_`.
    #[test]
    fn native_tests_hold_the_ci_filter() {
        let ci = include_str!("../../../../.github/workflows/ci.yml");
        assert!(ci.contains("cargo test -p ep9_vm --features spike-native --locked native\n"));
        let src = include_str!("layout6.rs");
        let body = &src[src.find("\nmod load_global {\n").expect("the module")..];
        let mut lines = body.lines();
        let mut tests = 0;
        while let Some(l) = lines.next() {
            if l.trim() == "#[test]" {
                let f = lines.next().unwrap_or_default().trim();
                let name = f.strip_prefix("fn ").expect("a test fn after #[test]");
                assert!(name.starts_with("native_"), "{name}");
                tests += 1;
            }
        }
        assert_eq!(tests, 4);
    }

    /// Tests run on parallel threads, so the offset of one test's stand-in must not be the
    /// offset of another. Thread A installs `lg` with its load offset. The test thread then
    /// sets offset 0 (what `install` does for `lt`, `lm`, `lz` and `nl`). After that, the
    /// call in thread A must still load at its own offset and give 7.
    #[test]
    fn native_offset_is_per_thread() {
        use std::sync::mpsc::channel;

        // Channels, not a barrier: if thread A fails before it sends, `recv` returns and
        // `join` reports the failure (a barrier would wait forever).
        let (installed, wait_installed) = channel::<()>();
        let (offset_set, wait_offset_set) = channel::<()>();
        let a = std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(move || {
                let mut iso = defs_isolate(config(Dispatch::Match, false));
                let off = offset_of(&iso, "lg", Opcode::LdaGlobal);
                assert!(off > 0, "the load follows a jump");
                let f = install(&iso, "lg", off);
                installed.send(()).unwrap();
                wait_offset_set.recv().unwrap();
                call(&mut iso, f, &[Value::FALSE]).map_err(|e| e.message)
            })
            .unwrap();
        if wait_installed.recv().is_ok() {
            set_off(0);
            offset_set.send(()).unwrap();
        }
        assert_eq!(a.join().unwrap(), Ok(Value::int32(7).to_bits()));
    }
}
