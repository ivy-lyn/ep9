//! OBJECT_LAYOUT 5, IC_LAYOUT 3 and VM_LAYOUT 1 (M5-P0, owner P0-VM5; plan v10 §2.3, §2.4): the
//! versions, the kind ids 81-83 (declared; registered by their builders or still free), the
//! elements-kind values, `Atom::Callee`, and the `crate::layout` export read on live objects
//! in the default build (C-18: the export is no longer behind `spike-native`).

use std::hint::black_box;

use ep9_bytecode::{FunctionId, VerifiedUnit};
use ep9_heap::{CellKind, CellSize, KindError, KindId};

use super::{Asm, config, isolate, meta, unit};
use crate::atoms::Atom;
use crate::elements::{self, Storage, kind as ek};
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::object::{self, arr, delems, realmf};
use crate::{Dispatch, IsolateConfig, kinds, layout, shape, versions};

fn dummy_trace(_: ep9_heap::CellPtr, _: &mut ep9_heap::Tracer<'_>) {}

fn dummy_kind(name: &'static str) -> CellKind {
    CellKind {
        name,
        size: CellSize::Fixed(16),
        trace: dummy_trace,
    }
}

// ---- versions -------------------------------------------------------------------------------

/// The month-5 versions (V10-D2): OBJECT_LAYOUT 4 -> 5, IC_LAYOUT 2 -> 3 and the new VM_LAYOUT 1
/// move once in M5-P0; FRAME 2, DEOPT_META 0 (V10-D29), the builtin ABI 1 and the bytecode ABI
/// 5 stay.
#[test]
fn month_5_versions() {
    assert_eq!(
        (
            black_box(versions::OBJECT_LAYOUT_VERSION),
            versions::IC_LAYOUT_VERSION,
            versions::VM_LAYOUT_VERSION
        ),
        (5, 3, 1)
    );
    assert_eq!(
        (
            versions::FRAME_VERSION,
            versions::BUILTIN_ABI_VERSION,
            versions::DEOPT_META_VERSION,
            ep9_bytecode::ABI_VERSION
        ),
        (2, 1, 0, 5)
    );
}

// ---- kinds ----------------------------------------------------------------------------------

#[test]
fn the_month_5_kind_ids() {
    assert_eq!(
        [kinds::ELEMENTS_DOUBLE, kinds::ELEMENTS_DICT, kinds::FUSE].map(KindId::get),
        [81, 82, 83]
    );
    assert_eq!(
        (
            layout::KIND_ELEMENTS,
            layout::KIND_ELEMENTS_DOUBLE,
            layout::KIND_ELEMENTS_DICT
        ),
        (65, 81, 82)
    );
}

/// M5-P0 registers none of 81-83 (their builders do: ELEM1, ELEM2, FR0 if a fuse needs a
/// cell). On a new isolate each is free, or taken by its builder; never another error.
#[test]
fn the_month_5_kinds_are_reserved_or_registered() {
    for id in [kinds::ELEMENTS_DOUBLE, kinds::ELEMENTS_DICT, kinds::FUSE] {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        match iso.rt.heap.register_kind(id, dummy_kind("test")) {
            Ok(()) | Err(KindError::Taken(_)) => {}
            Err(e) => panic!("kind {}: {e:?}", id.get()),
        }
    }
}

/// The elements-kind values, their storage forms and the IC_LAYOUT 3 kinds-set bits
/// (V10-A21, MC-w15-21).
#[test]
fn elements_kind_values_and_storage_forms() {
    assert_eq!(
        [
            ek::PACKED_INT32,
            ek::HOLEY_INT32,
            ek::PACKED_DOUBLE,
            ek::HOLEY_DOUBLE,
            ek::PACKED_ELEMENTS,
            ek::HOLEY_ELEMENTS,
            ek::DICTIONARY
        ],
        [0, 1, 2, 3, 4, 5, 6]
    );
    let forms: Vec<_> = (0..16).map(elements::storage).collect();
    use Storage::{Dictionary, Doubles, Values};
    assert_eq!(
        forms[..7],
        [
            Some(Values),
            Some(Values),
            Some(Doubles),
            Some(Doubles),
            Some(Values),
            Some(Values),
            Some(Dictionary)
        ]
    );
    assert!(forms[7..].iter().all(Option::is_none), "7-15 are reserved");
    let holey: Vec<u8> = (0..ek::COUNT).filter(|&k| elements::is_holey(k)).collect();
    assert_eq!(holey, [1, 3, 5]);
    // One bit per value 0-6 from bit 10; bits 0-9 keep their IC_LAYOUT 2 meaning.
    let bits: u32 = (0..ek::COUNT).map(elements::kinds_set_bit).sum();
    assert_eq!(bits, 0x7F << 10);
    assert_eq!(
        bits & (crate::profile::keyed::HOLE | crate::profile::keyed::OUT_OF_BOUNDS),
        0
    );
    // The shape bits: counts 28-31, free of the kind id (20-27).
    assert_eq!(shape::ELEMENTS_KIND_SHIFT, 28);
    assert_eq!(
        (ek::MAX as u32) << shape::ELEMENTS_KIND_SHIFT,
        shape::ELEMENTS_KIND_MASK as u32
    );
    assert_eq!(layout::SHAPE_ELEMENTS_KIND_SHIFT, 28);
    assert_eq!(layout::KEYED_KINDS_SET_SHIFT, 10);
    // Every array is HOLEY_ELEMENTS until ELEM1 builds the lattice.
    assert_eq!(elements::kind_of_array(UNDEFINED), ek::HOLEY_ELEMENTS);
}

/// The ELEMENTS_DOUBLE layout and its hole (V9-D39): a NaN that is not the canonical NaN, so
/// no DOUBLE store writes it.
#[test]
fn the_double_elements_layout() {
    assert_eq!((delems::CAPACITY, delems::DATA), (8, 16));
    assert_eq!(delems::HOLE, 0xFFF7_FFFF_FFF7_FFFF);
    assert!(f64::from_bits(delems::HOLE).is_nan());
    assert_eq!(delems::CANONICAL_NAN, f64::NAN.to_bits());
    assert_ne!(delems::HOLE, delems::CANONICAL_NAN);
    assert_eq!(
        (
            layout::ELEMENTS_DOUBLE_CAPACITY,
            layout::ELEMENTS_DOUBLE_DATA,
            layout::ELEMENTS_DOUBLE_HOLE
        ),
        (8, 16, delems::HOLE)
    );
}

/// The shape bit positions of OBJECT_LAYOUT 5: the slack counter in a root shape's flags bits
/// 4-7, the representation in details bits 4-6 and the const bit 7.
#[test]
fn the_shape_bit_positions() {
    assert_eq!(shape::flag::SLACK_MASK, 0xF0);
    let used = shape::flag::EXTENSIBLE
        | shape::flag::DICTIONARY
        | shape::flag::PROTOTYPE
        | shape::flag::UNIQUE;
    assert_eq!(used & shape::flag::SLACK_MASK, 0);
    assert_eq!((shape::rep::MASK, shape::DETAILS_CONST), (0x70, 0x80));
    assert_eq!(
        [
            shape::rep::NONE,
            shape::rep::INT32,
            shape::rep::DOUBLE,
            shape::rep::HEAP,
            shape::rep::ANY
        ],
        [0, 1, 2, 3, 4]
    );
}

/// `Atom::Callee` is a predefined atom (V10-D11), appended after the week-4 list (id 35).
#[test]
fn callee_is_a_predefined_atom() {
    assert_eq!((Atom::Callee.id(), Atom::Callee.text()), (35, "callee"));
    assert_eq!(Atom::ALL.last(), Some(&Atom::Callee));
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    rt.heap.collect();
    assert_eq!(
        crate::atoms::intern_str(rt, "callee"),
        rt.atom(Atom::Callee)
    );
}

// ---- layout facts on live objects --------------------------------------------------------------

/// The `Cx` offsets (VM_LAYOUT 1) read the live `Cx` of a default-build isolate: the three native
/// fields exist in every build; `runtime_table` is null without `spike-native`.
#[test]
fn the_cx_offsets_read_the_live_cx() {
    let iso = isolate(IsolateConfig::default());
    let base = core::ptr::from_ref::<crate::interp::Cx>(&iso.cx).cast::<u8>();
    // SAFETY: each offset names a word-sized field of the live `Cx`.
    unsafe {
        let word = |off: usize| base.add(off).cast::<usize>().read();
        assert_eq!(layout::CX_TABLE, 0);
        assert_eq!(word(layout::CX_TABLE), iso.cx.table[0] as usize);
        assert_eq!(word(layout::CX_STACK_LIMIT), iso.cx.stack_limit.addr());
        assert_eq!(
            base.add(layout::CX_BUDGET).cast::<i64>().read(),
            iso.cx.budget
        );
        assert_eq!(word(layout::CX_GLOBAL_THIS) as u64, iso.rt.global());
        assert_eq!(word(layout::CX_INTERRUPT), iso.cx.interrupt.addr());
        assert_eq!(word(layout::CX_NATIVE_LIMIT), iso.rt.native_limit);
        assert_ne!(word(layout::CX_NATIVE_LIMIT), 0);
        assert_eq!(
            word(layout::CX_RUNTIME_TABLE) == 0,
            !cfg!(feature = "spike-native")
        );
        assert_eq!(word(layout::CX_UNIT_DATA), 0);
        assert_eq!(word(layout::CX_OUT_FP), iso.cx.out_fp.addr());
        assert_eq!(word(layout::CX_OUT_PC), iso.cx.out_pc.addr());
    }
}

/// The RuntimeTable (VM_LAYOUT 1): one pointer per entry, in declaration order, 11 entries.
#[test]
fn the_runtime_table_offsets() {
    let rt = [
        layout::RT_POLL,
        layout::RT_CALL_ANY,
        layout::RT_ADD,
        layout::RT_INC,
        layout::RT_COMPARE,
        layout::RT_STACK_OVERFLOW,
        layout::RT_THROW,
        layout::RT_UNWIND,
        layout::RT_GET_KEYED,
        layout::RT_SET_KEYED,
        // Appended by M5-P0 (V10-D39): the deopt entry. 11 entries = VM_LAYOUT 1.
        layout::RT_DEOPT,
    ];
    for (i, off) in rt.into_iter().enumerate() {
        assert_eq!(off, i * 8);
    }
    assert_eq!(size_of::<layout::RuntimeTable>(), rt.len() * 8);
    assert_eq!(rt.len(), 11);
}

/// Arrays (ARRAY +32 allocation site, OBJECT_LAYOUT 5) and REALM (+48, +56) on live cells; the
/// ELEMENTS cell of an array has kind `KIND_ELEMENTS`.
#[test]
fn arrays_and_realms_on_live_cells() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let a = object::new_array(rt, 2);
    object::array_set(rt, a, 0, int32_bits(5));
    // SAFETY: `a`, its ELEMENTS cell and the realm are live cells of `iso`, which runs
    // nothing while this reads.
    unsafe {
        let kind = |c: u64| ((c as usize + layout::KIND_BYTE as usize) as *const u8).read();
        assert_eq!(kind(a), layout::KIND_ARRAY);
        assert_eq!(word_at(a, arr::ALLOC_SITE), UNDEFINED);
        assert_eq!(arr::ALLOC_SITE, arr::LENGTH + 8);
        assert_eq!(int32_of(word_at(a, layout::ARRAY_LENGTH as usize)), 1);
        let e = word_at(a, layout::ARRAY_ELEMENTS as usize);
        assert_eq!(kind(e), layout::KIND_ELEMENTS);
        assert_eq!(word_at(e, layout::ELEMENTS_DATA as usize), int32_bits(5));
        let realm = rt.root(crate::runtime::Root::Realm);
        assert_eq!(kind(realm), kinds::REALM.get());
        assert_eq!(
            (
                realmf::MAPPED_ARGUMENTS_SHAPE,
                realmf::UNMAPPED_ARGUMENTS_SHAPE,
                realmf::SIZE
            ),
            (48, 56, 64)
        );
        assert_eq!(word_at(realm, realmf::MAPPED_ARGUMENTS_SHAPE), UNDEFINED);
        assert_eq!(word_at(realm, realmf::UNMAPPED_ARGUMENTS_SHAPE), UNDEFINED);
    }
    // The array cell is 8 B bigger (2 fixed fields); the realm is 7 Values.
    assert_eq!(object::fixed_count(kinds::ARRAY.get()), 2);
}

/// A live CONTEXT cell read through the CONTEXT offsets (MC-w16-16).
#[test]
fn the_context_offsets_read_a_live_context() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let outer = object::new_context(rt, UNDEFINED, core::ptr::null(), 1);
    let inner = object::new_context(rt, outer, core::ptr::null(), 3);
    object::context_set(rt, inner, 2, int32_bits(9));
    // SAFETY: `outer` and `inner` are live CONTEXT cells; nothing runs.
    unsafe {
        assert_eq!(word_at(inner, layout::CONTEXT_PREVIOUS as usize), outer);
        assert_eq!(
            word_at(inner, layout::CONTEXT_SCOPE as usize),
            crate::frame::native_of(core::ptr::null::<u8>())
        );
        assert_eq!(int32_of(word_at(inner, layout::CONTEXT_LENGTH as usize)), 3);
        assert_eq!(
            word_at(inner, layout::CONTEXT_SLOTS as usize + 8 * 2),
            int32_bits(9)
        );
    }
}

/// `ld2(o)` = `o.b`, run with two receiver shapes: its LoadNamed slot holds a POLY_IC cell, and
/// `POLY_HANDLER` (MC-w16-4) reads each case's handler.
fn poly_program() -> VerifiedUnit {
    let mut ld = Asm::new(meta("ld2", 1, 0, 0, 0));
    let a0 = ld.b.parameter(0);
    ld.get_named(a0, "b");
    ld.b.return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("ld2");
    let (o1, o2) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(o1);
    s.b.lda_smi(7);
    s.set_named(o1, "a");
    s.b.lda_smi(8);
    s.set_named(o1, "b");
    s.b.ldar(o1);
    s.sta_global("o1");
    s.b.create_empty_object_literal().star(o2);
    s.b.lda_smi(1);
    s.set_named(o2, "b");
    s.b.ldar(o2);
    s.sta_global("o2");
    let f = s.f;
    for arg in [o1, o2] {
        s.lda_global("ld2");
        s.b.star(f).call_undefined_receiver1(f, arg, s.call);
    }
    s.b.lda_undefined().return_();
    let script = s.finish();
    unit("", vec![script, ld.finish()])
}

#[test]
fn poly_handler_reads_a_live_poly_ic() {
    let mut iso = isolate(IsolateConfig {
        feedback_threshold: 0,
        ..IsolateConfig::default()
    });
    iso.with_cx_checkpoint(|cx| cx.run_script(poly_program(), "poly.js").map(|_| ()))
        .unwrap();
    let g = |n: &str| iso.global_raw(n).unwrap().to_bits();
    let (o1, o2) = (g("o1"), g("o2"));
    let fi = object::function_info(g("ld2"));
    let v = crate::feedback::vector(fi).expect("a vector");
    let named = Asm::new(meta("x", 0, 0, 0, 0)).named.index();
    let smi =
        |off: u32| int32_bits(layout::SMI_FIELD_IN_OBJECT | (off << layout::SMI_SHIFT) as i32);
    // SAFETY: `v` and the POLY_IC cell are live cells of `iso`, which runs nothing.
    unsafe {
        let at = layout::FEEDBACK_SLOTS as usize + layout::FEEDBACK_SLOT_SIZE as usize * named;
        assert_eq!(word_at(v, at + layout::FEEDBACK_KEY as usize), layout::HOLE);
        let p = word_at(v, at + layout::FEEDBACK_VALUE as usize);
        assert_eq!(
            ((p as usize + layout::KIND_BYTE as usize) as *const u8).read(),
            kinds::POLY_IC.get()
        );
        let shape_id =
            |c: u64| ((c as usize + layout::SHAPE_ID_BYTE as usize) as *const u32).read();
        let case = |i: u32| {
            let at = (layout::POLY_CASES + layout::POLY_CASE_SIZE * i) as usize;
            (
                word_at(p, at) as u32,
                word_at(p, at + layout::POLY_HANDLER as usize),
            )
        };
        assert_eq!(layout::POLY_HANDLER, 8);
        assert_eq!(
            [case(0), case(1)],
            [
                (shape_id(o1), smi(layout::OBJECT_FIELDS + 8)),
                (shape_id(o2), smi(layout::OBJECT_FIELDS)),
            ]
        );
    }
}

/// The word at `addr + off`.
///
/// # Safety
/// `addr + off` is an 8-aligned word of a live cell, and the isolate runs nothing.
unsafe fn word_at(addr: u64, off: usize) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { ((addr as usize + off) as *const u64).read() }
}

/// IC_LAYOUT 3: the ALLOC_SITE handler kind is new and distinct; the IC_HANDLER cell stays 40 B.
#[test]
fn the_alloc_site_handler_kind() {
    use crate::ic::hk;
    let kinds = [
        hk::PROTO_LOAD,
        hk::MISSING,
        hk::ADD,
        hk::GLOBAL,
        hk::CONSTRUCT_SITE,
        hk::INSTANCEOF,
        hk::CLONE,
        hk::ALLOC_SITE,
    ];
    assert_eq!(kinds, [1, 2, 3, 4, 5, 6, 7, 8]);
}
