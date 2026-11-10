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
//! | `detach_zeroes_every_live_view` | 2 views, one dead before the GC: the list holds the live one; detach zeroes TypedArrays and DataViews |
//! | `transfer_moves_or_copies_the_store` | the BYTES cell moves with no copy; a resizable store moves; other lengths copy; the errors |
//! | `shared_buffers_grow_and_never_detach` | grow, the RangeErrors and TypeErrors, views on a SAB |
//! | `get_and_set_value_in_both_byte_orders` | each type, little and big endian, unaligned and shared; the range checks |
//! | `copy_bytes_with_overlap` | CopyDataBlockBytes forward, backward, across buffers and shared |
//! | `atomics_operations` | load, store, the 6 read-modify-write ops and compare-exchange on each integer type |
//! | `data_view_slots` | the DataView slots, its range checks, auto length |
//! | `a_failed_transfer_leaves_the_buffer` | ArrayBufferCopyAndDetach order: a failed grow of a moved store leaves the old buffer as it was |

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
use crate::{Dispatch, Isolate, kinds, layout};

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
        assert_eq!(
            object::slot_get(b, slot::TABLE),
            UNDEFINED,
            "no entry: no view"
        );
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
            let auto = view(&iso.rt, 1, b, 2, None);
            let dv = ab::new_data_view(&iso.rt, NULL, b, 1, Some(4)).unwrap();
            let dv_auto = ab::new_data_view(&iso.rt, NULL, b, 0, None).unwrap();
            let gone = below_a_pad(|| dead_view(&mut iso, b));
            assert_eq!(iso.rt.buffers.views_of(b).len(), 5);
            scrub_stack();
            iso.collect_garbage();
            let views = iso.rt.buffers.views_of(b);
            assert_eq!(
                views,
                [live, auto, dv, dv_auto],
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
            assert!(data(live).is_null() && data(auto).is_null());
            assert_eq!(ta::fixed_length(live), Some(0));
            assert_eq!(
                ta::fixed_length(auto),
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
        assert_eq!(object::slot_get(dv, dv_slot::FLAGS), int32_bits(0));
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
