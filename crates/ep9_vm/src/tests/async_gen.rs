//! Async generators in the VM (M3-P2 ASYNCGEN; `generator.rs`, `interp/slow/async_gen.rs`):
//! the request queue and the async-from-sync iterator are heap cells that a GC traces (plan
//! rule 8), the queue is first in, first out, and the runtime ids 36-41 reach
//! `slow/async_gen.rs` under every dispatch (they replace the M3-P0 stubs, whose routing
//! `tests/layout3.rs` checked). JS-level tests are in `crates/ep9_builtins/tests/async_gen.rs`
//! (ep9_vm tests cannot compile JS).

use std::hint::black_box;

use ep9_bytecode::{RuntimeId, VerifiedUnit};
use ep9_heap::GlobalHandle;

use super::{Asm, config, isolate, on_big_stack, run_in, unit};
use crate::Dispatch;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits};
use crate::generator::{self, MODE_ASYNC_FROM_SYNC, agf};
use crate::object::{self, cell, genf};
use crate::runtime::{Runtime, val};

/// Masks the bits of a cell pointer, so the conservative stack scan cannot keep it.
const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frames that make the cells (the pattern of
/// tests/layout3.rs; tests/mod.rs is frozen, so each file has its own copy).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> R {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    black_box(&mut pad);
    r
}

/// True if a live cell starts at the masked address `masked`.
#[inline(never)]
fn is_live(rt: &Runtime, masked: u64) -> bool {
    rt.heap.is_cell(cell(black_box(masked) ^ MASK).addr())
}

/// An async generator object (no frame: `FUNCTION` is undefined) with its record and three
/// requests whose value and promise are fresh objects, and an async-from-sync iterator over
/// a fresh iterator and `next`. Both rooted by global handles. Returns the handles and the
/// masked bits of the fresh objects.
#[inline(never)]
fn made_cells(rt: &Runtime) -> (Vec<GlobalHandle>, Vec<(String, u64)>) {
    let mut fresh = Vec::new();
    let record = generator::new_async_generator_record(rt);
    let g = object::new_generator(
        rt,
        NULL,
        UNDEFINED,
        UNDEFINED,
        UNDEFINED,
        genf::MODE_ASYNC_GENERATOR,
        1,
    );
    object::gen_set(rt, g, genf::ASYNC, record);
    let modes = [genf::RESUME_NEXT, genf::RESUME_RETURN, genf::RESUME_THROW];
    for (i, mode) in modes.into_iter().enumerate() {
        let v = object::new_plain_object(rt);
        let p = object::new_plain_object(rt);
        generator::ag_enqueue(rt, g, mode, v, p);
        fresh.push((format!("request {i} value"), black_box(v) ^ MASK));
        fresh.push((format!("request {i} promise"), black_box(p) ^ MASK));
    }
    let it = object::new_plain_object(rt);
    let next = object::new_plain_object(rt);
    let afs = generator::new_async_from_sync_iterator(rt, it, next);
    fresh.push(("async-from-sync iterator".into(), black_box(it) ^ MASK));
    fresh.push(("async-from-sync next".into(), black_box(next) ^ MASK));
    let handles = vec![rt.heap.new_global(val(g)), rt.heap.new_global(val(afs))];
    (handles, fresh)
}

/// Plan rule 8: the record, every request cell and the async-from-sync iterator are heap
/// cells that a GC traces: a value only they hold survives a collection (with and without GC
/// stress), the queue keeps its order, and the heap verifies.
#[test]
fn the_request_queue_and_the_async_from_sync_iterator_are_traced_heap_cells() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.record_conservative_roots = true;
            let iso = isolate(c);
            let rt: &Runtime = &iso.rt;
            let (handles, fresh) = below_a_pad(|| made_cells(rt));
            scrub_stack();
            rt.heap.collect();
            let css = rt.heap.last_conservative_roots();
            for (what, m) in &fresh {
                let addr = cell(m ^ MASK).addr();
                assert!(
                    !css.iter().any(|c| c.addr() == addr),
                    "{what}: the conservative scan kept it"
                );
            }
            for (what, m) in &fresh {
                assert!(
                    below_a_pad(|| is_live(rt, *m)),
                    "{what} was traced (stress={stress})"
                );
            }
            iso.verify_heap().unwrap();
            let g = rt.heap.global(handles[0]).to_bits();
            let afs = rt.heap.global(handles[1]).to_bits();
            assert!(generator::is_async_generator(g));
            assert_eq!(generator::ag_state(g), agf::STATE_SUSPENDED_START);
            assert_eq!(object::gen_get_i32(afs, genf::MODE), MODE_ASYNC_FROM_SYNC);
            // First in, first out: the promises come back in request order.
            for i in 0..3 {
                let p = generator::ag_dequeue(rt, g).expect("a request");
                assert_eq!(p ^ MASK, fresh[2 * i + 1].1, "request {i}");
            }
            assert_eq!(generator::ag_dequeue(rt, g), None);
            let record = object::gen_get(g, genf::ASYNC);
            assert_eq!(object::list_get(record, agf::HEAD), UNDEFINED);
            assert_eq!(object::list_get(record, agf::TAIL), UNDEFINED);
            for h in handles {
                rt.heap.drop_global(h);
            }
        });
    }
}

/// A script that calls runtime id `id` with Smis 1.. as its arguments (`next` = the hole for
/// id 41's GetIterator form when `hole`).
fn call_id(id: RuntimeId, hole: bool) -> VerifiedUnit {
    let argc = id.info().arity.expect("fixed arity");
    let mut s = Asm::script(0);
    let args = s.b.new_register_list(argc);
    for i in 0..argc {
        s.b.lda_smi(i as i32 + 1).star(args.get(i));
    }
    if hole {
        s.b.lda_the_hole().star(args.get(1));
    }
    s.b.call_runtime(id, args, argc).return_();
    unit("", vec![s.finish()])
}

/// Ids 36-41 reach `slow/async_gen.rs` (the M3-P0 stubs are gone): called outside an async
/// generator, ids 36-40 throw their internal error, id 41 needs an object iterator, and its
/// GetIterator form (the hole, `async_gen.rs` AG3) throws the "not async iterable" TypeError
/// for a number. Every dispatch, with and without GC stress.
#[test]
fn ids_36_to_41_reach_async_gen() {
    let cases: [(RuntimeId, bool, &str); 7] = [
        (
            RuntimeId::AsyncGeneratorEnter,
            false,
            "Error: ep9 internal error: AsyncGeneratorEnter outside its async generator function",
        ),
        (
            RuntimeId::AsyncGeneratorAwait,
            false,
            "Error: ep9 internal error: AsyncGeneratorAwait outside a running async generator",
        ),
        (
            RuntimeId::AsyncGeneratorYield,
            false,
            "Error: ep9 internal error: AsyncGeneratorYield outside a running async generator",
        ),
        (
            RuntimeId::AsyncGeneratorResolve,
            false,
            "Error: ep9 internal error: AsyncGeneratorResolve outside a running async generator",
        ),
        (
            RuntimeId::AsyncGeneratorReject,
            false,
            "Error: ep9 internal error: AsyncGeneratorReject outside a running async generator",
        ),
        (
            RuntimeId::CreateAsyncFromSyncIterator,
            false,
            "Error: ep9 internal error: CreateAsyncFromSyncIterator of a non-object iterator",
        ),
        (
            RuntimeId::CreateAsyncFromSyncIterator,
            true,
            "TypeError: 1 is not async iterable",
        ),
    ];
    for (id, hole, want) in cases {
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let r = on_big_stack(move || run_in(config(d, stress), call_id(id, hole)));
                let e = r.result.expect_err("throws");
                assert_eq!(
                    e.to_string(),
                    format!("test.js:1:1: {want}"),
                    "{} {} stress={stress}",
                    id.info().name,
                    d.name()
                );
            }
        }
    }
}

/// The hole is never a JS value, so it can mark id 41's GetIterator form (AG3).
#[test]
fn the_hole_is_not_a_js_value() {
    assert_ne!(HOLE, UNDEFINED);
    assert_ne!(HOLE, NULL);
    assert_ne!(HOLE, int32_bits(0));
}
