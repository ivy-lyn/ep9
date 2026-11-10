//! ArrayBuffer and SharedArrayBuffer data (TypedArray TA-1 TA-VM; design
//! `docs/plan/notes/typedarray-design.md` §2.3, §5; needs N1): the BYTES cell, the off-heap
//! stores, the BufferTable and its weak source, the external bytes, detach, transfer, resize and
//! grow, the byte access of GetValueFromBuffer and SetValueInBuffer, CopyDataBlockBytes, the
//! Atomics operations and the DataView slots. Rust-level tests; each runs with GC stress off and
//! on (stress: a GC at every allocation and at every `add_external_bytes`, so a table borrow
//! held across that call would panic in the weak source), then the heap verifier.
//!
//! | Test | What it shows |
//! |---|---|
//! | `a_fixed_buffer_is_a_bytes_cell` | zeroed bytes at cell + 16, no store at length 0, the length caps |
//! | `a_buffer_the_heap_cannot_give_is_a_range_error` | a RangeError past `max_heap_bytes` and when the heap is full, no abort |
//! | `the_external_bytes_go_up_and_down` | resizable and shared stores: the count after alloc, grow, shrink, detach and the death of the buffer (minor and major GC); the dead store is dropped after the GC |
//! | `detach_zeroes_every_live_view` | 2 views, one dead before the GC: the list holds the live one; detach zeroes TypedArrays and DataViews; a TYPED_VARIABLE view gets a null data word and keeps its auto length |
//! | `transfer_moves_or_copies_the_store` | the BYTES cell moves with no copy; a resizable store moves; other lengths copy; the errors |
//! | `shared_buffers_grow_and_never_detach` | grow, the RangeErrors and TypeErrors, views on a SAB |
//! | `get_and_set_value_in_both_byte_orders` | each type, little and big endian, unaligned and shared; the range checks |
//! | `copy_bytes_with_overlap` | CopyDataBlockBytes forward, backward, across buffers and shared |
//! | `atomics_operations` | load, store, the 6 read-modify-write ops and compare-exchange on each integer type |
//! | `data_view_slots` | the DataView slots (slot 4: its own table entry), its range checks, auto length |
//! | `a_failed_transfer_leaves_the_buffer` | ArrayBufferCopyAndDetach order: a failed grow of a moved store leaves the old buffer as it was |
//! | `every_engine_object_owns_its_entry_and_no_other_does` | the brand (TA-1 fixer r3): each maker's buffer or DataView owns a table entry from birth, a caller object never does (also with a copied index), the entries of dead ones go in every GC kind; 10 000 make and drop rounds with GCs: no caller object at a reused address is branded |
//! | `the_slot_api_refuses_the_store_slot` | safe `Cx` only: `set_slot` and `slot` of a buffer store or a DataView data word panic (Codex TA-1 r1 f1_e) |
//! | `the_slot_api_refuses_the_byte_length_slot` | safe `Cx` only: `set_slot` of the byte length panics (f1_a, f1_b, f1_d); the range checks hold |
//! | `the_slot_api_refuses_the_other_engine_slots` | safe `Cx` only: every engine slot of a buffer or a DataView refuses a write; the value slots stay readable |
//! | `the_slot_api_refuses_the_table_slot` | safe `Cx` only: the table index cannot move to another buffer (f1_f, f1_g) |
//! | `an_object_of_a_buffer_class_from_the_slot_api_is_not_a_buffer` | safe `Cx` only: `new_object_with_slots` of a buffer class is a TypeError in every `abi/typed.rs` method (f1_c) |

use std::hint::black_box;

use super::typed_array::{buffer, view};
use super::{config, isolate, on_big_stack};
use crate::abi::ObjectClass;
use crate::array_buffer::{self as ab, Fail, dv_slot, slot};
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::object;
use crate::runtime::{Runtime, val};
use crate::typed_array as ta;
use crate::{Dispatch, Isolate, Local, kinds, layout};

fn each(test: fn(&mut Cx)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let r = iso.with_cx(|cx| {
                test(cx.inner());
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

fn n(v: u64) -> f64 {
    val(v).as_number().expect("a Number")
}

fn num(d: f64) -> u64 {
    crate::conv::num(d)
}

fn shared(rt: &Runtime, len: u64, max: Option<u64>) -> u64 {
    ab::allocate_shared(rt, NULL, len, max).unwrap()
}

/// The bytes of the buffer `b` (non-shared), read through its store.
fn bytes_of(rt: &Runtime, b: u64) -> Vec<u8> {
    let len = ab::byte_length(rt, b);
    let p = ab::data_base(rt, b);
    // SAFETY: `b` is live and its store holds `len` bytes.
    (0..len).map(|i| unsafe { p.add(i).read() }).collect()
}

fn write_bytes(rt: &Runtime, b: u64, bytes: &[u8]) {
    for (i, &x) in bytes.iter().enumerate() {
        ab::set_value(rt, b, i as u64, 1, int32_bits(i32::from(x)), true).unwrap();
    }
}

#[test]
fn a_fixed_buffer_is_a_bytes_cell() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 13, None);
        assert_eq!(object::slot_class(b), Some(ObjectClass::ArrayBuffer.id()));
        let store = object::slot_get(b, slot::STORE);
        assert_eq!(kinds::cell_kind(store), kinds::BYTES.get());
        // Its own table entry from birth (the brand, TA-1 fixer r3), with no view yet.
        let i = val(object::slot_get(b, slot::TABLE)).as_int32().unwrap();
        assert!(rt.buffers.owns(i as usize, b), "an entry with no view");
        assert!(rt.buffers.views_of(b).is_empty());
        assert_eq!(object::slot_get(b, slot::MAX_BYTE_LENGTH), UNDEFINED);
        assert_eq!(ab::byte_length(rt, b), 13);
        let c = object::cell(store);
        assert_eq!(
            n(object::rd(c, layout::BYTES_LENGTH as usize)),
            13.0,
            "+8 the byte length"
        );
        // SAFETY: `store` is a live BYTES cell of this heap.
        let size = unsafe { rt.heap.cell_size(c) };
        assert_eq!(
            size,
            layout::BYTES_DATA as usize + 16,
            "16 + 13 rounded to 8"
        );
        assert_eq!(
            ab::data_base(rt, b).addr(),
            c.addr() + layout::BYTES_DATA as usize
        );
        assert_eq!(bytes_of(rt, b), vec![0; 13], "zeroed at birth");
        // A large one is a large-object cell, zeroed too.
        let big = buffer(cx.rt(), 64 << 10, None);
        assert!(bytes_of(cx.rt(), big).iter().all(|&x| x == 0));
        // Length 0: no store; the data address is null.
        let zero = buffer(cx.rt(), 0, None);
        assert_eq!(object::slot_get(zero, slot::STORE), UNDEFINED);
        assert!(ab::data_base(cx.rt(), zero).is_null());
        let v = view(cx.rt(), 0, zero, 0, Some(0));
        assert_eq!(ta::length(cx.rt(), v), Some(0));
        // The caps of design §2.3: maxByteLength above 2^32; length above the maximum.
        assert_eq!(
            ab::allocate(cx.rt(), NULL, 0, Some(ab::MAX_RESERVATION + 1)),
            Err(Fail::Range("Invalid array buffer max length"))
        );
        assert_eq!(
            ab::allocate(cx.rt(), NULL, 9, Some(8)),
            Err(Fail::Range("Invalid array buffer length"))
        );
        assert!(ab::allocate(cx.rt(), NULL, 0, Some(ab::MAX_RESERVATION)).is_ok());
    });
}

/// CreateByteDataBlock that the heap cannot give (above `max_heap_bytes`, or after the
/// collection and the near-limit callback): a RangeError, no abort; the heap still works.
#[test]
fn a_buffer_the_heap_cannot_give_is_a_range_error() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.max_heap_bytes = 64 << 20;
            let mut iso = isolate(c);
            let rt: &Runtime = &iso.rt;
            let kept = buffer(rt, 40 << 20, None);
            // Past the limit; then a heap that is full after its collection.
            for len in [(64 << 20) + 1, 1 << 62, u64::MAX, 40 << 20] {
                assert_eq!(
                    ab::allocate(rt, NULL, len, None),
                    Err(Fail::Range("Array buffer allocation failed")),
                    "{len}"
                );
            }
            let b = buffer(rt, 1 << 20, None);
            assert_eq!(ab::byte_length(rt, b), 1 << 20);
            assert_eq!(ab::byte_length(rt, black_box(kept)), 40 << 20);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

// ---- dead buffers: made below a pad, so no copy of their addresses stays where the
// conservative scan reads (the tests/regexp.rs form) -----------------------------------------

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;
const DEEP: usize = 64 << 10;

#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> R {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    black_box(&mut pad);
    r
}

/// Makes 4 resizable and 4 shared buffers (each with a view) that nothing keeps; returns
/// their masked addresses.
#[inline(never)]
fn dead_buffers(iso: &mut Isolate) -> Vec<usize> {
    let rt: &Runtime = &iso.rt;
    let mut out = Vec::new();
    for i in 0..4u64 {
        let b = buffer(rt, 100 + i, Some(1 << 20));
        black_box(view(rt, 1, b, 0, None));
        out.push(black_box(b as usize) ^ MASK);
        let s = shared(rt, 100 + i, None);
        black_box(view(rt, 1, s, 0, None));
        out.push(black_box(s as usize) ^ MASK);
    }
    out
}

#[test]
fn the_external_bytes_go_up_and_down() {
    // No minor GC under stress: the stress GCs make the dead buffers old before it runs.
    for (stress, minor) in [(false, false), (false, true), (true, false)] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.generational = true;
            c.heap.record_conservative_roots = true;
            let mut iso = isolate(c);
            let page = ep9_platform::va::page_size();
            let ext = |iso: &Isolate| iso.rt.heap.external_bytes();
            assert_eq!(ext(&iso), 0);
            // A kept resizable buffer: its committed pages count; grow, shrink and detach move
            // the count.
            let kept = buffer(&iso.rt, 10, Some(16 * page as u64));
            assert_eq!(ext(&iso), page);
            ab::resize(&iso.rt, kept, 3 * page as u64 + 1).unwrap();
            assert_eq!(ext(&iso), 4 * page);
            ab::resize(&iso.rt, kept, page as u64).unwrap();
            assert_eq!(ext(&iso), page);
            ab::resize(&iso.rt, kept, 0).unwrap();
            assert_eq!(ext(&iso), 0);
            ab::resize(&iso.rt, kept, 5).unwrap();
            assert_eq!(ext(&iso), page);
            let sab = shared(&iso.rt, 1, Some(4 * page as u64));
            assert_eq!(ext(&iso), 2 * page);
            ab::grow_shared(&iso.rt, sab, 2 * page as u64).unwrap();
            assert_eq!(ext(&iso), 3 * page);
            ab::detach(&iso.rt, kept, UNDEFINED).unwrap();
            assert_eq!(ext(&iso), 2 * page, "detach drops the store's bytes");
            let entries = iso.rt.buffers.live_entries();
            // Buffers that die: the weak source removes their bytes and frees their entries.
            let cells = below_a_pad(|| dead_buffers(&mut iso));
            assert_eq!(ext(&iso), 2 * page + 8 * page);
            assert_eq!(iso.rt.buffers.live_entries(), entries + 8);
            scrub_stack();
            if minor {
                iso.rt.heap.collect_minor();
            } else {
                iso.collect_garbage();
            }
            let css = iso.rt.heap.last_conservative_roots();
            for &m in &cells {
                let addr = m ^ MASK;
                assert!(
                    !css.iter().any(|c| c.addr() == addr),
                    "the scan kept {addr:#x}"
                );
            }
            assert_eq!(ext(&iso), 2 * page, "the dead buffers' bytes are gone");
            assert_eq!(iso.rt.buffers.live_entries(), entries);
            assert_eq!(iso.rt.buffers.dead_stores(), 8, "dropped after the GC");
            // The next table call drops them (the unmap runs outside the GC).
            let b = buffer(&iso.rt, 1, Some(1));
            assert_eq!(iso.rt.buffers.dead_stores(), 0);
            assert_eq!(ext(&iso), 3 * page);
            black_box((b, sab, cells.len()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A view on `b` that nothing keeps.
#[inline(never)]
fn dead_view(iso: &mut Isolate, b: u64) -> usize {
    black_box(view(&iso.rt, 0, b, 0, Some(2)) as usize) ^ MASK
}

#[test]
fn detach_zeroes_every_live_view() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let b = buffer(&iso.rt, 8, None);
            let live = view(&iso.rt, 5, b, 4, Some(1));
            let computed = view(&iso.rt, 1, b, 2, None);
            // A TYPED_VARIABLE view (no length, a resizable buffer): detach nulls its data word
            // too, and its length field stays auto (reviewer F1; design §5.2 step 2).
            let rab = buffer(&iso.rt, 8, Some(64));
            let var = view(&iso.rt, 1, rab, 2, None);
            let dv = ab::new_data_view(&iso.rt, NULL, b, 1, Some(4)).unwrap();
            let dv_auto = ab::new_data_view(&iso.rt, NULL, b, 0, None).unwrap();
            let gone = below_a_pad(|| dead_view(&mut iso, b));
            assert_eq!(iso.rt.buffers.views_of(b).len(), 5);
            scrub_stack();
            iso.collect_garbage();
            let views = iso.rt.buffers.views_of(b);
            assert_eq!(
                views,
                [live, computed, dv, dv_auto],
                "the dead view left the list"
            );
            assert!(!views.iter().any(|&v| v as usize == gone ^ MASK));
            let data = |v: u64| {
                crate::external_ref::ExternalRef::<u8>::from_word(object::rd(
                    object::cell(v),
                    layout::VIEW_DATA as usize,
                ))
                .get()
            };
            let base = ab::data_base(&iso.rt, b);
            assert_eq!(data(live), base.wrapping_add(4).cast_const());
            ab::detach(&iso.rt, b, UNDEFINED).unwrap();
            assert!(data(live).is_null() && data(computed).is_null());
            assert_eq!(ta::fixed_length(live), Some(0));
            assert_eq!(
                ta::fixed_length(computed),
                Some(0),
                "no length on a fixed-length buffer made a fixed view (length 6)"
            );
            for (v, len) in [(dv, int32_bits(0)), (dv_auto, int32_bits(-1))] {
                let w = object::slot_get(v, dv_slot::DATA);
                let p = crate::external_ref::ExternalRef::<u8>::from_word(w).get();
                assert!(p.is_null());
                assert_eq!(object::slot_get(v, dv_slot::BYTE_LENGTH), len);
            }
            assert!(iso.rt.buffers.views_of(b).is_empty());
            assert_eq!(object::slot_get(b, slot::STORE), UNDEFINED);
            assert_eq!(ab::byte_length(&iso.rt, b), 0);
            assert!(ab::is_detached(b));
            assert_eq!(
                crate::elements::kind_of(var),
                crate::elements::kind::TYPED_VARIABLE
            );
            assert_eq!(
                data(var),
                ab::data_base(&iso.rt, rab).wrapping_add(2).cast_const()
            );
            assert!(!data(var).is_null());
            ab::detach(&iso.rt, rab, UNDEFINED).unwrap();
            assert!(
                data(var).is_null(),
                "a detached TYPED_VARIABLE view has no data"
            );
            assert_eq!(ta::fixed_length(var), None);
            assert_eq!(ta::length(&iso.rt, var), None);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

#[test]
fn transfer_moves_or_copies_the_store() {
    each(|cx| {
        let rt = cx.rt();
        // Same length, fixed: the BYTES cell moves with no copy.
        let b = buffer(rt, 6, None);
        write_bytes(rt, b, &[1, 2, 3, 4, 5, 6]);
        let v = view(rt, 1, b, 0, Some(6));
        let cell = object::slot_get(b, slot::STORE);
        let nb = ab::copy_and_detach(cx.rt(), b, NULL, 6, false).unwrap();
        let rt = cx.rt();
        assert_eq!(
            object::slot_get(nb, slot::STORE),
            cell,
            "the same BYTES cell"
        );
        assert_eq!(bytes_of(rt, nb), [1, 2, 3, 4, 5, 6]);
        assert!(ab::is_detached(b) && !ab::is_detached(nb));
        assert_eq!(ta::length(rt, v), None);
        // Other lengths copy min(old, new) bytes; new bytes are 0.
        let longer = ab::copy_and_detach(rt, nb, NULL, 8, false).unwrap();
        assert_eq!(bytes_of(cx.rt(), longer), [1, 2, 3, 4, 5, 6, 0, 0]);
        let shorter = ab::copy_and_detach(cx.rt(), longer, NULL, 2, true).unwrap();
        assert_eq!(bytes_of(cx.rt(), shorter), [1, 2]);
        assert_eq!(
            ab::max_byte_length(shorter),
            None,
            "a fixed buffer stays fixed"
        );
        // A resizable store that stays resizable moves (its base stays), then takes the length.
        let rt = cx.rt();
        let r = buffer(rt, 4, Some(64));
        write_bytes(rt, r, &[9, 8, 7, 6]);
        let base = ab::data_base(rt, r);
        let ext = rt.heap.external_bytes();
        let moved = ab::copy_and_detach(rt, r, NULL, 2, true).unwrap();
        let rt = cx.rt();
        assert_eq!(ab::data_base(rt, moved), base);
        assert_eq!(ab::max_byte_length(moved), Some(64));
        assert_eq!(bytes_of(rt, moved), [9, 8]);
        assert_eq!(rt.heap.external_bytes(), ext, "the moved bytes count once");
        ab::resize(rt, moved, 4).unwrap();
        assert_eq!(bytes_of(cx.rt(), moved), [9, 8, 0, 0]);
        // transferToFixedLength of a resizable buffer copies into a fixed one.
        let fixed = ab::copy_and_detach(cx.rt(), moved, NULL, 3, false).unwrap();
        assert_eq!(ab::max_byte_length(fixed), None);
        assert_eq!(bytes_of(cx.rt(), fixed), [9, 8, 0]);
        // The errors, in spec order.
        let rt = cx.rt();
        let detached = ab::copy_and_detach(rt, b, NULL, 0, false);
        assert_eq!(
            detached,
            Err(Fail::Type("Cannot transfer a detached ArrayBuffer"))
        );
        let s = shared(rt, 4, None);
        assert!(matches!(
            ab::copy_and_detach(rt, s, NULL, 4, false),
            Err(Fail::Type(_))
        ));
        let r2 = buffer(rt, 4, Some(8));
        assert_eq!(
            ab::copy_and_detach(rt, r2, NULL, 9, true),
            Err(Fail::Range("Invalid array buffer length"))
        );
        assert!(!ab::is_detached(r2), "a RangeError leaves the buffer");
        let o = object::new_plain_object(rt);
        assert!(matches!(
            ab::copy_and_detach(rt, o, NULL, 0, false),
            Err(Fail::Type(_))
        ));
        // A buffer with a detach key: transfer and a detach with another key fail.
        let k = buffer(rt, 2, None);
        object::slot_set(rt, k, slot::DETACH_KEY, int32_bits(7));
        assert!(matches!(
            ab::copy_and_detach(rt, k, NULL, 2, false),
            Err(Fail::Type(_))
        ));
        assert!(matches!(ab::detach(rt, k, UNDEFINED), Err(Fail::Type(_))));
        ab::detach(rt, k, int32_bits(7)).unwrap();
        assert!(ab::is_detached(k));
    });
}

#[test]
fn shared_buffers_grow_and_never_detach() {
    each(|cx| {
        let rt = cx.rt();
        let s = shared(rt, 4, Some(64));
        assert_eq!(
            object::slot_class(s),
            Some(ObjectClass::SharedArrayBuffer.id())
        );
        assert!(ab::is_shared(s) && ab::is_resizable(s));
        let base = ab::data_base(rt, s);
        let auto = view(rt, 5, s, 0, None);
        let fixed = view(rt, 5, s, 0, Some(1));
        assert_eq!(ta::length(rt, auto), Some(1));
        let k = super::ic::key(rt, "0");
        crate::property::set(cx, fixed, k, int32_bits(-7), true).unwrap();
        let rt = cx.rt();
        ab::grow_shared(rt, s, 16).unwrap();
        assert_eq!(ab::byte_length(rt, s), 16);
        assert_eq!(ab::data_base(rt, s), base);
        assert_eq!(ta::length(rt, auto), Some(4));
        assert_eq!(ta::length(rt, fixed), Some(1));
        assert_eq!(ab::get_value(rt, s, 0, 5, true).unwrap(), int32_bits(-7));
        assert_eq!(ab::get_value(rt, s, 12, 5, true).unwrap(), int32_bits(0));
        ab::grow_shared(rt, s, 16).unwrap();
        assert_eq!(
            ab::grow_shared(rt, s, 8),
            Err(Fail::Range("Invalid array buffer length: it cannot shrink"))
        );
        assert_eq!(
            ab::grow_shared(rt, s, 65),
            Err(Fail::Range("Invalid array buffer length"))
        );
        assert!(matches!(ab::detach(rt, s, UNDEFINED), Err(Fail::Type(_))));
        assert!(matches!(ab::resize(rt, s, 8), Err(Fail::Type(_))));
        let fixed_sab = shared(rt, 8, None);
        assert!(!ab::is_resizable(fixed_sab));
        assert!(matches!(
            ab::grow_shared(rt, fixed_sab, 8),
            Err(Fail::Type(_))
        ));
        let r = buffer(rt, 1, Some(2));
        assert!(matches!(ab::grow_shared(rt, r, 2), Err(Fail::Type(_))));
        assert_eq!(
            ab::allocate_shared(rt, NULL, 9, Some(8)),
            Err(Fail::Range("Invalid array buffer length"))
        );
        assert_eq!(
            ab::allocate_shared(rt, NULL, 0, Some(ab::MAX_RESERVATION + 1)),
            Err(Fail::Range("Invalid array buffer max length"))
        );
        assert!(!ab::is_detached(s));
    });
}

/// (element type, a value, its little-endian bytes): each Table 71 type.
fn byte_rows(rt: &Runtime) -> Vec<(u8, u64, Vec<u8>)> {
    vec![
        (0, int32_bits(-2), vec![0xFE]),
        (1, int32_bits(200), vec![200]),
        (2, num(254.5), vec![254]),
        (3, int32_bits(-2), vec![0xFE, 0xFF]),
        (4, int32_bits(0x1234), vec![0x34, 0x12]),
        (5, int32_bits(-2), vec![0xFE, 0xFF, 0xFF, 0xFF]),
        (6, num(4294967294.0), vec![0xFE, 0xFF, 0xFF, 0xFF]),
        (
            7,
            crate::bigint::from_i128(rt, -2),
            vec![0xFE; 1].into_iter().chain([0xFF; 7]).collect(),
        ),
        (
            8,
            crate::bigint::from_i128(rt, 0x0102_0304_0506_0708),
            vec![8, 7, 6, 5, 4, 3, 2, 1],
        ),
        (9, num(1.0), vec![0x00, 0x3C]),
        (10, num(1.0), vec![0x00, 0x00, 0x80, 0x3F]),
        (11, num(1.0), vec![0, 0, 0, 0, 0, 0, 0xF0, 0x3F]),
    ]
}

#[test]
fn get_and_set_value_in_both_byte_orders() {
    each(|cx| {
        for is_shared in [false, true] {
            let rt = cx.rt();
            let b = if is_shared {
                shared(rt, 11, None)
            } else {
                buffer(rt, 11, None)
            };
            for (t, v, le) in byte_rows(cx.rt()) {
                let rt = cx.rt();
                let size = le.len();
                assert_eq!(size, ab::elem_size(t));
                // At an odd index: an unaligned access (a byte loop on shared bytes).
                for (at, little) in [(0u64, true), (3, true), (3, false)] {
                    ab::set_value(rt, b, at, t, v, little).unwrap();
                    let got: Vec<u8> = (0..size)
                        .map(|i| n(ab::get_value(rt, b, at + i as u64, 1, true).unwrap()) as u8)
                        .collect();
                    let mut want = le.clone();
                    if !little {
                        want.reverse();
                    }
                    assert_eq!(got, want, "type {t} at {at} little {little}");
                    let back = ab::get_value(rt, b, at, t, little).unwrap();
                    if kinds::is_bigint(v) {
                        assert!(crate::bigint::equals(back, v), "type {t}");
                    } else if t == 2 {
                        assert_eq!(n(back), 254.0);
                    } else {
                        assert_eq!(n(back), n(v), "type {t}");
                    }
                }
                // The range checks: the last element that fits, one past it; wrong types.
                let last = 11 - size as u64;
                assert!(ab::get_value(rt, b, last, t, true).is_ok());
                assert_eq!(
                    ab::get_value(rt, b, last + 1, t, true).map(drop),
                    Err(Fail::Range("Offset is outside the bounds of the buffer"))
                );
                assert!(ab::get_value(rt, b, u64::MAX, t, true).is_err());
                let wrong = if ab::is_bigint_elem(t) {
                    int32_bits(1)
                } else {
                    UNDEFINED
                };
                assert!(matches!(
                    ab::set_value(rt, b, 0, t, wrong, true),
                    Err(Fail::Type(_))
                ));
            }
        }
        let rt = cx.rt();
        let d = buffer(rt, 4, None);
        ab::detach(rt, d, UNDEFINED).unwrap();
        assert!(matches!(
            ab::get_value(rt, d, 0, 0, true),
            Err(Fail::Type(_))
        ));
    });
}

#[test]
fn copy_bytes_with_overlap() {
    each(|cx| {
        for is_shared in [false, true] {
            let rt = cx.rt();
            let make = |rt: &Runtime, len| {
                if is_shared {
                    shared(rt, len, None)
                } else {
                    buffer(rt, len, None)
                }
            };
            let b = make(rt, 8);
            write_bytes(rt, b, &[1, 2, 3, 4, 5, 6, 7, 8]);
            let read = |rt: &Runtime, b: u64| -> Vec<u8> {
                (0..ab::byte_length(rt, b) as u64)
                    .map(|i| n(ab::get_value(rt, b, i, 1, true).unwrap()) as u8)
                    .collect()
            };
            ab::copy_bytes(rt, b, 2, b, 0, 5).unwrap();
            assert_eq!(read(rt, b), [1, 2, 1, 2, 3, 4, 5, 8], "forward overlap");
            ab::copy_bytes(rt, b, 0, b, 3, 5).unwrap();
            assert_eq!(read(rt, b), [2, 3, 4, 5, 8, 4, 5, 8], "backward overlap");
            let c = make(rt, 3);
            ab::copy_bytes(rt, c, 1, b, 6, 2).unwrap();
            assert_eq!(read(rt, c), [0, 5, 8]);
            ab::copy_bytes(rt, c, 3, b, 8, 0).unwrap();
            assert!(matches!(
                ab::copy_bytes(rt, c, 2, b, 0, 2),
                Err(Fail::Range(_))
            ));
            assert!(matches!(
                ab::copy_bytes(rt, c, 0, b, 7, 2),
                Err(Fail::Range(_))
            ));
            assert_eq!(read(rt, c), [0, 5, 8], "a failed copy writes nothing");
        }
    });
}

#[test]
fn atomics_operations() {
    each(|cx| {
        for is_shared in [false, true] {
            let rt = cx.rt();
            let b = if is_shared {
                shared(rt, 16, None)
            } else {
                buffer(rt, 16, None)
            };
            for t in [0u8, 1, 3, 4, 5, 6, 7, 8] {
                let rt = cx.rt();
                let size = ab::elem_size(t) as u64;
                let val_of = |x: i64| -> u64 {
                    if ab::is_bigint_elem(t) {
                        crate::bigint::from_i128(rt, i128::from(x))
                    } else {
                        num(x as f64)
                    }
                };
                let as_i = |v: u64| -> i128 {
                    if kinds::is_bigint(v) {
                        crate::bigint::to_i128(v).unwrap()
                    } else {
                        n(v) as i128
                    }
                };
                let at = 8;
                ab::atomic_store(rt, b, at, t, val_of(12)).unwrap();
                assert_eq!(as_i(ab::atomic_load(rt, b, at, t).unwrap()), 12);
                // (op, operand, the element after it), each returning the old element.
                let mut cur = 12i128;
                for (op, x, after) in [
                    (0u8, 5i64, 17i128),
                    (1, 0b10101, 17 & 0b10101),
                    (2, 6, 6),
                    (3, 9, 15),
                    (4, 20, -5),
                    (5, 3, -5 ^ 3),
                ] {
                    let old = ab::atomic_rmw(rt, b, at, t, op, val_of(x)).unwrap();
                    assert_eq!(as_i(old), cur, "type {t} op {op}");
                    let now = as_i(ab::atomic_load(rt, b, at, t).unwrap());
                    let bits = 8 * size as u32;
                    let wrap = |v: i128| -> i128 {
                        let m = v.rem_euclid(1i128 << bits);
                        let signed = matches!(t, 0 | 3 | 5 | 7);
                        if signed && m >= 1i128 << (bits - 1) {
                            m - (1i128 << bits)
                        } else {
                            m
                        }
                    };
                    assert_eq!(now, wrap(after), "type {t} op {op}");
                    cur = now;
                }
                // compareExchange: no change on a mismatch, a change on a match.
                let old = ab::atomic_compare_exchange(rt, b, at, t, val_of(99), val_of(1)).unwrap();
                assert_eq!(as_i(old), cur);
                assert_eq!(as_i(ab::atomic_load(rt, b, at, t).unwrap()), cur);
                let old = ab::atomic_compare_exchange(rt, b, at, t, val_of(cur as i64), val_of(1))
                    .unwrap();
                assert_eq!(as_i(old), cur);
                assert_eq!(as_i(ab::atomic_load(rt, b, at, t).unwrap()), 1);
                if size > 1 {
                    assert_eq!(
                        ab::atomic_load(rt, b, at + 1, t).map(drop),
                        Err(Fail::Range("Invalid atomic access index"))
                    );
                }
                assert!(matches!(
                    ab::atomic_rmw(rt, b, at, t, 6, val_of(1)),
                    Err(Fail::Type(_))
                ));
            }
            let rt = cx.rt();
            for t in [2u8, 9, 10, 11] {
                assert!(matches!(ab::atomic_load(rt, b, 0, t), Err(Fail::Type(_))));
            }
            assert!(matches!(ab::atomic_load(rt, b, 16, 5), Err(Fail::Range(_))));
        }
    });
}

#[test]
fn data_view_slots() {
    each(|cx| {
        let rt = cx.rt();
        let b = buffer(rt, 8, Some(16));
        let dv = ab::new_data_view(rt, NULL, b, 2, Some(4)).unwrap();
        assert_eq!(object::slot_class(dv), Some(ObjectClass::DataView.id()));
        assert_eq!(object::slot_get(dv, dv_slot::BUFFER), b);
        assert_eq!(n(object::slot_get(dv, dv_slot::BYTE_OFFSET)), 2.0);
        assert_eq!(n(object::slot_get(dv, dv_slot::BYTE_LENGTH)), 4.0);
        // Slot 4 names the DataView's own table entry (the brand), not its buffer's.
        let j = val(object::slot_get(dv, dv_slot::TABLE))
            .as_int32()
            .expect("an int32 index");
        assert!(
            rt.buffers.owns(j as usize, dv),
            "the DataView owns entry {j}"
        );
        assert!(ab::is_data_view(rt, dv));
        assert_ne!(object::slot_get(b, slot::TABLE), int32_bits(j));
        let w = object::slot_get(dv, dv_slot::DATA);
        let p = crate::external_ref::ExternalRef::<u8>::from_word(w).get();
        assert_eq!(p, ab::data_base(rt, b).wrapping_add(2).cast_const());
        let auto = ab::new_data_view(rt, NULL, b, 8, None).unwrap();
        assert_eq!(
            object::slot_get(auto, dv_slot::BYTE_LENGTH),
            int32_bits(layout::VIEW_LENGTH_AUTO)
        );
        for (off, len) in [(9, None), (6, Some(3)), (0, Some(u64::MAX))] {
            assert!(matches!(
                ab::new_data_view(rt, NULL, b, off, len),
                Err(Fail::Range(_))
            ));
        }
        let o = object::new_plain_object(rt);
        assert!(matches!(
            ab::new_data_view(rt, NULL, o, 0, None),
            Err(Fail::Type(_))
        ));
        // The views' slot lists in the table: both DataViews.
        assert_eq!(rt.buffers.views_of(b), [dv, auto]);
        black_box(ta::length);
    });
}

/// ArrayBufferCopyAndDetach (§25.1.3.3) makes the new buffer (step 7) before it detaches the
/// old one: when the grow of a moved resizable store fails, the old buffer keeps its store,
/// its bytes, its views and its external bytes, and a later transfer works. The failure is
/// injected at the external-bytes add of the grow (`Heap::add_external_bytes` fails when the
/// count would overflow; the commit has no test hook, and its failure takes the same `Err`
/// path of `array_buffer::resize`).
#[test]
fn a_failed_transfer_leaves_the_buffer() {
    each(|cx| {
        let rt = cx.rt();
        let r = buffer(rt, 4, Some(4 << 20));
        write_bytes(rt, r, &[1, 2, 3, 4]);
        let v = view(rt, 1, r, 0, None);
        let base = ab::data_base(rt, r);
        let ext = rt.heap.external_bytes();
        // The next add of 2 bytes or more overflows the count. A GC here frees no store: the
        // isolate has no dead buffer.
        let big = usize::MAX - ext - 1;
        rt.heap.add_external_bytes(big).unwrap();
        let grow = ab::copy_and_detach(rt, r, NULL, 2 << 20, true);
        let rt = cx.rt();
        rt.heap.remove_external_bytes(big);
        assert_eq!(
            grow,
            Err(Fail::Range("Array buffer allocation failed")),
            "the grow fails"
        );
        assert!(!ab::is_detached(r), "a failed transfer does not detach");
        assert_eq!(ab::data_base(rt, r), base);
        assert_eq!(bytes_of(rt, r), [1, 2, 3, 4]);
        assert_eq!(
            rt.heap.external_bytes(),
            ext,
            "the store's bytes count once"
        );
        assert_eq!(ta::length(rt, v), Some(4));
        let k = super::ic::key(rt, "2");
        assert_eq!(crate::property::get(cx, v, k).unwrap(), int32_bits(3));
        assert_eq!(cx.rt().buffers.views_of(r), [v]);
        // The same transfer with room: the store moves, then `r` is detached.
        let moved = ab::copy_and_detach(cx.rt(), r, NULL, 8, true).unwrap();
        let rt = cx.rt();
        assert!(ab::is_detached(r));
        assert_eq!(ta::length(rt, v), None);
        assert_eq!(ab::data_base(rt, moved), base);
        assert_eq!(bytes_of(rt, moved), [1, 2, 3, 4, 0, 0, 0, 0]);
        // A shrink that cannot fail: the same order.
        let shrunk = ab::copy_and_detach(rt, moved, NULL, 2, true).unwrap();
        assert!(ab::is_detached(moved));
        assert_eq!(bytes_of(cx.rt(), shrunk), [1, 2]);
    });
}

// ---- the brand: every engine object owns its table entry (TA-1 fixer r3, design study) ------

/// Makes a viewless fixed buffer, a zero-length buffer and a DataView on `keep` that nothing
/// keeps; returns their masked addresses.
#[inline(never)]
fn dead_engine_objects(iso: &mut Isolate, keep: u64) -> Vec<usize> {
    let rt: &Runtime = &iso.rt;
    let fixed = buffer(rt, 8, None);
    let empty = buffer(rt, 0, None);
    let dv = ab::new_data_view(rt, NULL, keep, 0, None).unwrap();
    [fixed, empty, dv]
        .iter()
        .map(|&o| black_box(o as usize) ^ MASK)
        .collect()
}

/// The table slot of an engine object of class `class` (5 a buffer, 4 a DataView) and its
/// slot count.
fn table_slot(class: u16) -> (usize, usize) {
    if class == ObjectClass::DataView.id() {
        (dv_slot::TABLE, dv_slot::COUNT)
    } else {
        (slot::TABLE, slot::COUNT)
    }
}

/// The design study's T8 (judge §4.5; free.md T6): `is_engine_object` holds for the result of
/// each maker (fixed, zero-length, resizable, SAB, DataView, both transfer forms and the copy,
/// detached buffers) and for nothing else, a caller object with a copied table index included.
/// A viewless buffer, a zero-length buffer and a DataView own entries, which go when they die
/// (major and minor GC). Then 10 000 rounds (2 000 under GC stress) make and drop engine and
/// caller objects with GCs between them: a caller object at the address of a dead engine
/// object, with that object's old table index, is never branded (the weak source frees an
/// entry in the GC that finds its owner dead, before the sweep can give the address again).
/// Under GC stress most caller objects get such an address; the test checks that some do.
#[test]
fn every_engine_object_owns_its_entry_and_no_other_does() {
    // No minor GC under stress: the stress GCs make the dead objects old before it runs.
    for (stress, minor) in [(false, false), (false, true), (true, false)] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.generational = true;
            c.heap.record_conservative_roots = true;
            let mut iso = isolate(c);
            let rt: &Runtime = &iso.rt;
            let fixed = buffer(rt, 8, None);
            let empty = buffer(rt, 0, None);
            let rab = buffer(rt, 8, Some(64));
            let sab = shared(rt, 8, Some(64));
            let dv = ab::new_data_view(rt, NULL, fixed, 0, None).unwrap();
            let moved = ab::copy_and_detach(rt, buffer(rt, 8, None), NULL, 8, false).unwrap();
            let moved_rab =
                ab::copy_and_detach(rt, buffer(rt, 8, Some(64)), NULL, 16, true).unwrap();
            let old = buffer(rt, 8, None);
            let copied = ab::copy_and_detach(rt, old, NULL, 4, false).unwrap();
            let detached = buffer(rt, 8, Some(16));
            ab::detach(rt, detached, UNDEFINED).unwrap();
            let engine = [
                fixed, empty, rab, sab, dv, moved, moved_rab, old, copied, detached,
            ];
            for (n, &o) in engine.iter().enumerate() {
                assert!(ab::is_engine_object(rt, o), "engine object {n}");
                let class = object::slot_class(o).unwrap();
                let (t, _) = table_slot(class);
                let i = val(object::slot_get(o, t)).as_int32().unwrap();
                assert!(rt.buffers.owns(i as usize, o), "engine object {n}");
                assert_eq!(ab::is_data_view(rt, o), class == ObjectClass::DataView.id());
                assert_eq!(ab::buffer_class(rt, o).is_some(), !ab::is_data_view(rt, o));
            }
            assert!(ab::is_detached(old) && ab::is_detached(detached));
            // Caller objects of the same classes and counts (and others), with the table index
            // of a real one copied in: no brand.
            for (n, &real) in engine.iter().enumerate() {
                let class = object::slot_class(real).unwrap();
                let (t, count) = table_slot(class);
                let idx = object::slot_get(real, t);
                for k in [count, 0, 3, 7] {
                    let o = object::new_slot_object(rt, NULL, class, k);
                    if t < k {
                        object::slot_set(rt, o, t, idx);
                    }
                    assert!(!ab::is_engine_object(rt, o), "copy of {n}, {k} slots");
                    assert!(ab::buffer_class(rt, o).is_none() && !ab::is_data_view(rt, o));
                }
            }
            let plain = object::new_plain_object(rt);
            for v in [plain, UNDEFINED, NULL, int32_bits(0), num(1.5)] {
                assert!(!ab::is_engine_object(rt, v));
            }
            // A viewless buffer, a zero-length buffer and a DataView die: their 3 entries go.
            let entries = rt.buffers.live_entries();
            let cells = below_a_pad(|| dead_engine_objects(&mut iso, fixed));
            assert_eq!(iso.rt.buffers.live_entries(), entries + 3);
            assert_eq!(iso.rt.buffers.views_of(fixed).len(), 2);
            scrub_stack();
            if minor {
                iso.rt.heap.collect_minor();
            } else {
                iso.collect_garbage();
            }
            let css = iso.rt.heap.last_conservative_roots();
            for &m in &cells {
                let addr = m ^ MASK;
                assert!(
                    !css.iter().any(|c| c.addr() == addr),
                    "the scan kept {addr:#x}"
                );
            }
            assert_eq!(iso.rt.buffers.live_entries(), entries);
            assert_eq!(iso.rt.buffers.views_of(fixed), [dv]);
            // Make and drop: a caller object never takes the brand of a dead engine object
            // whose address it got (it gets that object's old table index too).
            let rt: &Runtime = &iso.rt;
            // GC stress (a GC at each allocation) gives the freed addresses back at once; with
            // no stress the allocator takes fresh holes first, so few are reused.
            let rounds = if stress { 2_000 } else { 10_000 };
            let mut index_at = std::collections::HashMap::new();
            let mut reused = 0usize;
            for r in 0..rounds {
                let e = match r % 3 {
                    0 => buffer(rt, 8, None),
                    1 => buffer(rt, 0, None),
                    _ => ab::new_data_view(rt, NULL, fixed, 0, Some(4)).unwrap(),
                };
                assert!(ab::is_engine_object(rt, e), "round {r}");
                let class = object::slot_class(e).unwrap();
                let (t, count) = table_slot(class);
                index_at.insert(e, object::slot_get(e, t));
                let o = object::new_slot_object(rt, NULL, class, count);
                let idx = match index_at.get(&o) {
                    Some(&i) => {
                        reused += 1;
                        i
                    }
                    None => object::slot_get(e, t),
                };
                object::slot_set(rt, o, t, idx);
                assert!(!ab::is_engine_object(rt, o), "round {r}");
                black_box((e, o));
                if r % 10 == 9 {
                    if (r / 10) % 2 == 0 {
                        rt.heap.collect_minor();
                    } else {
                        rt.heap.collect();
                    }
                }
            }
            eprintln!(
                "stress {stress}: {rounds} rounds, {reused} caller objects at the address of an \
                 earlier engine object"
            );
            assert!(!stress || reused > 0, "no address was reused");
            black_box((empty, rab, sab, dv, moved, moved_rab, copied, cells.len()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

// ---- the slot API cannot forge a buffer (Codex TA-1 r1 finding 1) -----------------------------
//
// Each test below uses only safe public calls of the builtin ABI (`crate::abi::Cx`, `Local`), as
// an embedder or a builtin crate can. A forged slot would bound a raw access of the store, so
// the slot API refuses each one before any write (a panic: a bug in the caller), and an object
// that `new_object_with_slots` makes of a buffer class is not a buffer (a TypeError).

type Api<'a, 's> = &'a mut crate::abi::Cx<'s>;

/// True if `f` panics in the public `Cx` of a fresh isolate (GC stress as `stress`).
fn api_panics(stress: bool, f: impl for<'s> FnOnce(Api<'_, 's>) + Send + 'static) -> bool {
    on_big_stack(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let _ = iso.with_cx(|cx| {
                f(cx);
                Ok(())
            });
        }));
        r.is_err()
    })
}

/// Runs `f` with the public `Cx` of a fresh isolate, GC stress off and on, then the heap
/// verifier.
fn each_api(f: for<'s> fn(Api<'_, 's>)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let r = iso.with_cx(|cx| {
                f(cx);
                Ok(())
            });
            assert!(r.is_ok());
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// The error name of `r` ("TypeError", "RangeError"), or "ok".
fn error_name<T>(cx: Api<'_, '_>, r: ep9_value::JsResult<T>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(t) => {
            let e = cx.catch_exception(t).unwrap();
            let n = cx.get(e.as_object().unwrap(), "name").unwrap();
            cx.to_rust_string(n).unwrap()
        }
    }
}

/// The buffers of one slot test: a fixed one with a BYTES store, a zero-length one, a resizable
/// one (an off-heap store), a growable SharedArrayBuffer and a DataView, made by the ABI.
fn api_buffers<'s>(cx: Api<'_, 's>) -> [(Local<'s, crate::JsObject>, ObjectClass); 5] {
    let fixed = cx.allocate_array_buffer(Local::null(), 8, None).unwrap();
    let empty = cx.allocate_array_buffer(Local::null(), 0, None).unwrap();
    let rab = cx
        .allocate_array_buffer(Local::null(), 8, Some(4096))
        .unwrap();
    let sab = cx
        .allocate_shared_array_buffer(Local::null(), 8, Some(1 << 22))
        .unwrap();
    let dv = cx.new_data_view(Local::null(), fixed, 0, Some(8)).unwrap();
    let [a, b, c, d, e] = API_CLASSES;
    [(fixed, a), (empty, b), (rab, c), (sab, d), (dv, e)]
}

/// The classes of [`api_buffers`], in its order.
const API_CLASSES: [ObjectClass; 5] = [
    ObjectClass::ArrayBuffer,
    ObjectClass::ArrayBuffer,
    ObjectClass::ArrayBuffer,
    ObjectClass::SharedArrayBuffer,
    ObjectClass::DataView,
];

/// The slots of a class that only the engine writes, and the one it does not let safe code
/// read (it holds no JS value).
fn engine_slots(class: ObjectClass) -> (usize, usize) {
    match class {
        ObjectClass::DataView => (dv_slot::COUNT, dv_slot::DATA),
        _ => (slot::COUNT, slot::STORE),
    }
}

/// Site class "slot 0, the store" (`data_base`, `checked_range`, `view_data`, the copy of
/// `copy_and_detach`, the data word of every view): `set_slot` and `slot` of slot 0 panic for
/// a fixed, a zero-length, a resizable and a shared buffer (f1_e: undefined in slot 0 let the
/// GC free the bytes under a live view). Slot 3 of a DataView (its data address) likewise.
#[test]
fn the_slot_api_refuses_the_store_slot() {
    for stress in [false, true] {
        for which in 0..5 {
            let write = move |cx: Api<'_, '_>| {
                let (o, class) = api_buffers(cx)[which];
                let (_, word) = engine_slots(class);
                let _ = cx.set_slot(o, class, word, Local::undefined());
            };
            assert!(
                api_panics(stress, write),
                "set_slot of the store, case {which}"
            );
            let read = move |cx: Api<'_, '_>| {
                let (o, class) = api_buffers(cx)[which];
                let (_, word) = engine_slots(class);
                let _ = cx.slot(o, class, word);
            };
            assert!(api_panics(stress, read), "slot of the store, case {which}");
        }
    }
    // With no forge, a view's bytes live as long as the view (the f1_e steps, no set_slot).
    each_api(|cx| {
        let b = cx.allocate_array_buffer(Local::null(), 64, None).unwrap();
        let v = cx.new_typed_array(Local::null(), 1, b, 0, None).unwrap();
        cx.set_value_in_buffer(b, 3, 1, Local::int32(0xcd), true)
            .unwrap();
        let g = cx.global();
        cx.set(g, "v", v, true).unwrap();
        assert_eq!(cx.typed_array_length(v).unwrap(), Some(64));
        assert_eq!(cx.get(v, "3").unwrap().as_number(), Some(205.0));
    });
}

/// Site class "slot 1, the byte length" (`byte_length`, `checked_range`, `new_view`,
/// `new_data_view`, the bound of a length-tracking view): `set_slot` of slot 1 panics (f1_a:
/// a 0-length buffer with length 1 wrote through null; f1_b, f1_d: an 8-byte store with length
/// 4096). Reads of slot 1 still give the length. The range checks then hold.
#[test]
fn the_slot_api_refuses_the_byte_length_slot() {
    for stress in [false, true] {
        for which in 0..5 {
            let f = move |cx: Api<'_, '_>| {
                let (o, class) = api_buffers(cx)[which];
                let _ = cx.set_slot(o, class, 1, Local::number(4096.0));
            };
            assert!(api_panics(stress, f), "set_slot 1, case {which}");
        }
    }
    each_api(|cx| {
        let [(fixed, c), (empty, _), ..] = api_buffers(cx);
        assert_eq!(cx.slot(fixed, c, 1).unwrap().as_number(), Some(8.0));
        let r = cx.set_value_in_buffer(empty, 0, 1, Local::int32(7), true);
        assert_eq!(error_name(cx, r), "RangeError");
        let r = cx.set_value_in_buffer(fixed, 8, 1, Local::int32(7), true);
        assert_eq!(error_name(cx, r), "RangeError");
        let r = cx.get_value_from_buffer(fixed, 4000, 1, true);
        assert_eq!(error_name(cx, r), "RangeError");
        let v = cx
            .new_typed_array(Local::null(), 1, fixed, 0, None)
            .unwrap();
        assert_eq!(cx.typed_array_length(v).unwrap(), Some(8));
        let r = cx.new_typed_array(Local::null(), 1, fixed, 0, Some(4096));
        assert_eq!(error_name(cx, r), "RangeError");
    });
}

/// Site class "slot 4, the flags" (`is_detached`, `is_resizable`; every check above): a
/// `set_slot` of slot 4 panics (clearing the detached bit would give a null store a length),
/// and so does every other engine slot of a buffer (2 the maximum, 3 the detach key) and of a
/// DataView (0-4). Slots 1-5 of a buffer and 0-2 of a DataView stay readable; slot 4 of a
/// DataView (its table index) is not read.
#[test]
fn the_slot_api_refuses_the_other_engine_slots() {
    for stress in [false, true] {
        for (which, &class) in API_CLASSES.iter().enumerate() {
            for i in 0..engine_slots(class).0 {
                let f = move |cx: Api<'_, '_>| {
                    let (o, class) = api_buffers(cx)[which];
                    let _ = cx.set_slot(o, class, i, Local::int32(0));
                };
                assert!(api_panics(stress, f), "set_slot {i}, case {which}");
            }
        }
        let flags = |cx: Api<'_, '_>| {
            let (dv, class) = api_buffers(cx)[4];
            let _ = cx.slot(dv, class, dv_slot::TABLE);
        };
        assert!(api_panics(stress, flags), "slot 4 of a DataView");
    }
    each_api(|cx| {
        let bufs = api_buffers(cx);
        for (o, class) in bufs {
            let (count, word) = engine_slots(class);
            let last = if class == ObjectClass::DataView {
                dv_slot::BYTE_LENGTH
            } else {
                count - 1
            };
            for i in (0..=last).filter(|&i| i != word) {
                cx.slot(o, class, i).unwrap();
            }
        }
        let (fixed, dv) = (bufs[0].0, bufs[4].0);
        let b = cx.slot(dv, ObjectClass::DataView, dv_slot::BUFFER).unwrap();
        assert_eq!(b.bits(), fixed.bits());
        cx.detach_array_buffer(fixed, Local::undefined()).unwrap();
        assert!(cx.is_detached_buffer(fixed));
        let r = cx.get_value_from_buffer(fixed, 0, 1, true);
        assert_eq!(error_name(cx, r), "TypeError");
    });
}

/// Site class "slot 5, the table index" (`byte_length` and `shared_block` of a SAB,
/// `detach_unchecked` and `take_store`, `resize`, `grow_shared`, `add_view`): the index is
/// readable (an int32), but a `set_slot` of it panics (f1_f: a small SAB with a big SAB's
/// index; f1_g: a fixed buffer with a resizable one's index, then a detach).
#[test]
fn the_slot_api_refuses_the_table_slot() {
    for stress in [false, true] {
        let f1_f = |cx: Api<'_, '_>| {
            let small = cx
                .allocate_shared_array_buffer(Local::null(), 8, None)
                .unwrap();
            let big = cx
                .allocate_shared_array_buffer(Local::null(), 1 << 22, None)
                .unwrap();
            let i = cx
                .slot(big, ObjectClass::SharedArrayBuffer, slot::TABLE)
                .unwrap();
            assert!(i.as_int32().is_some());
            let _ = cx.set_slot(small, ObjectClass::SharedArrayBuffer, slot::TABLE, i);
        };
        assert!(api_panics(stress, f1_f), "f1_f");
        let f1_g = |cx: Api<'_, '_>| {
            let r = cx
                .allocate_array_buffer(Local::null(), 4096, Some(8192))
                .unwrap();
            let a = cx.allocate_array_buffer(Local::null(), 8, None).unwrap();
            let i = cx.slot(r, ObjectClass::ArrayBuffer, slot::TABLE).unwrap();
            let _ = cx.set_slot(a, ObjectClass::ArrayBuffer, slot::TABLE, i);
        };
        assert!(api_panics(stress, f1_g), "f1_g");
    }
    each_api(|cx| {
        let small = cx
            .allocate_shared_array_buffer(Local::null(), 8, Some(64))
            .unwrap();
        let big = cx
            .allocate_shared_array_buffer(Local::null(), 1 << 22, None)
            .unwrap();
        assert_eq!(cx.array_buffer_byte_length(small).unwrap(), 8);
        assert_eq!(cx.array_buffer_byte_length(big).unwrap(), 1 << 22);
        let r = cx.get_value_from_buffer(small, 1 << 21, 1, true);
        assert_eq!(error_name(cx, r), "RangeError");
        cx.shared_array_buffer_grow(small, 16).unwrap();
        assert_eq!(cx.array_buffer_byte_length(small).unwrap(), 16);
        let r = cx
            .allocate_array_buffer(Local::null(), 16, Some(64))
            .unwrap();
        let a = cx.allocate_array_buffer(Local::null(), 8, None).unwrap();
        cx.detach_array_buffer(a, Local::undefined()).unwrap();
        assert!(!cx.is_detached_buffer(r));
        cx.array_buffer_resize(r, 32).unwrap();
        cx.set_value_in_buffer(r, 31, 1, Local::int32(9), true)
            .unwrap();
        assert_eq!(
            cx.get_value_from_buffer(r, 31, 1, true)
                .unwrap()
                .as_number(),
            Some(9.0)
        );
    });
}

/// Site class "a buffer made from nothing" (f1_c): `new_object_with_slots` refuses the slot
/// count of a buffer or a DataView of `abi/typed.rs` for these classes. With any other count the
/// object and its slots are the caller's (as `tests::slots` uses them), but no `abi/typed.rs`
/// method takes it for a buffer: each gives a TypeError (or false), never an access, also after
/// the caller wrote the f1_c values (a length of 16, flags 0) into its slots.
#[test]
fn an_object_of_a_buffer_class_from_the_slot_api_is_not_a_buffer() {
    each_api(|cx| {
        let real = cx.allocate_array_buffer(Local::null(), 8, None).unwrap();
        for class in API_CLASSES.into_iter().skip(2) {
            for n in [0, 3, 5, 7, ObjectClass::MAX_SLOTS] {
                if n == engine_slots(class).0 {
                    continue;
                }
                let o = cx.new_object_with_slots(Local::null(), class, n);
                assert_eq!(cx.object_class(o), Some(class));
                for (i, x) in [(1, Local::number(16.0)), (4, Local::int32(0))] {
                    if i < n {
                        cx.set_slot(o, class, i, x).unwrap();
                        assert_eq!(cx.slot(o, class, i).unwrap().bits(), x.bits());
                    }
                }
                let r = cx.array_buffer_byte_length(o);
                assert_eq!(error_name(cx, r), "TypeError", "byte length, {n} slots");
                assert!(!cx.is_detached_buffer(o));
                let r = cx.detach_array_buffer(o, Local::undefined());
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.array_buffer_copy_and_detach(o, Local::null(), 8, true);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.array_buffer_resize(o, 8);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.shared_array_buffer_grow(o, 8);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.copy_buffer_bytes(real, 0, o, 0, 1);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.copy_buffer_bytes(o, 0, real, 0, 1);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.get_value_from_buffer(o, 0, 1, true);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.set_value_in_buffer(o, 0, 1, Local::int32(7), true);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.new_typed_array(Local::null(), 1, o, 0, None);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.new_data_view(Local::null(), o, 0, None);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.atomic_load(o, 0, 1);
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.atomic_store(o, 0, 1, Local::int32(7));
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.atomic_read_modify_write(o, 0, 1, 0, Local::int32(7));
                assert_eq!(error_name(cx, r), "TypeError");
                let r = cx.atomic_compare_exchange(o, 0, 1, Local::int32(0), Local::int32(7));
                assert_eq!(error_name(cx, r), "TypeError");
            }
        }
    });
    // The slot count of an `abi/typed.rs` object is refused (f1_c made a 6-slot ArrayBuffer).
    for class in API_CLASSES.into_iter().skip(2) {
        let f = move |cx: Api<'_, '_>| {
            let _ = cx.new_object_with_slots(Local::null(), class, engine_slots(class).0);
        };
        assert!(api_panics(false, f), "{class:?}");
    }
}
