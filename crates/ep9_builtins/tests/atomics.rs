//! The Atomics object at the JS level (TypedArray TA-4; `src/atomics.rs`; ECMA-262
//! `sec-atomics-object`). R1 (the skeleton) pins `%Atomics%` (its keys in node v26.13.1's
//! order, the attributes, names and lengths, `@@toStringTag`, the global key `Atomics`), the
//! lazy group (the key `Atomics` joins the group of `ArrayBuffer`, TA-D14: realm init makes no
//! `%Atomics%`). R2 ATOMICS appends the function tests: each op, the conversions and their
//! order, detach and shrink in a `valueOf`, `wait`, `waitAsync` and `notify` on one isolate and
//! across isolates on two threads (a shared block), `[[CanBlock]]` false, the `waitAsync`
//! promise in a microtask spin and in a host loop, and the drop of an isolate with async
//! waiters. Each single-isolate script prints the lines of node v26.13.1 for the same script
//! (`print` here, `console.log` there) unless its doc names a difference, and runs with and
//! without GC stress.
//!
//! ep9 resolves a notified waiter at the next job boundary (the host-task choice, M6-ORCH-601;
//! `ep9_vm` `abi/typed.rs`, `queue_head_after_async_waiters`): a notify puts the waiter of a
//! `waitAsync` promise into its agent's inbox. On Codex's trace S1 (waitAsync; queue A, which
//! queues B, which queues C; then notify) that gives the spec order A,W,B,C (node: A,B,C,W);
//! on S2 (a job calls notify, then queues B) and S3 (notify, then queue a job) it gives node's
//! order, not the spec's: ECMA-262 resolves a waiter of the same agent inside
//! `Atomics.notify`, and V8 posts a task (`futex-emulation.cc:185-210`).
//! `wait_async_resolves_in_notify_order` pins node's order (job, r3, r4, r5; the spec gives
//! r3, r4, job, r5).

mod common;

use std::hint::black_box;
use std::time::{Duration, Instant};

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::{
    Args, BuiltinModule, Cx, Exception, IntrinsicId, Isolate, JsResult, Local, SharedBlockRef,
};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `%Atomics%`: an ordinary extensible object with prototype `%Object.prototype%`, the global
/// key `Atomics` {w, c}, the 14 functions {w, c} in node's key order with their names and
/// lengths, and `@@toStringTag` "Atomics" {c}. No function is a constructor.
#[test]
fn atomics_has_the_spec_keys_and_attributes() {
    let src = r#"
const d = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  return ("value" in x ? "d" + (x.writable ? "w" : "-") : "a") + (x.enumerable ? "e" : "-") +
    (x.configurable ? "c" : "-");
};
print(typeof Atomics, d(globalThis, "Atomics"), Object.getPrototypeOf(Atomics) === Object.prototype,
  Object.isExtensible(Atomics), String(Atomics), Atomics[Symbol.toStringTag],
  d(Atomics, Symbol.toStringTag));
print(Reflect.ownKeys(Atomics).map((k) => {
  const v = Atomics[k];
  return String(k) + " " + d(Atomics, k) + (typeof v === "function" ? " " + v.name + " " + v.length : "");
}).join("\n"));
let isCtor = 0;
for (const k of Object.getOwnPropertyNames(Atomics)) {
  try { new Atomics[k](); } catch (e) { if (e instanceof TypeError) isCtor++; }
}
print(isCtor);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "object dw-c true true [object Atomics] Atomics d--c",
            "load dw-c load 2\n\
             store dw-c store 3\n\
             add dw-c add 3\n\
             sub dw-c sub 3\n\
             and dw-c and 3\n\
             or dw-c or 3\n\
             xor dw-c xor 3\n\
             exchange dw-c exchange 3\n\
             compareExchange dw-c compareExchange 4\n\
             isLockFree dw-c isLockFree 1\n\
             wait dw-c wait 4\n\
             waitAsync dw-c waitAsync 4\n\
             notify dw-c notify 3\n\
             pause dw-c pause 0\n\
             Symbol(Symbol.toStringTag) d--c",
            "14",
        ])
    );
}

/// Each load, store and read-modify-write function on each integer width: the values, the
/// wrap, the result (the old element; `store` gives the converted value: +0 for -0, the
/// integer part, +∞ as is), BigInt64 and BigUint64 (mod 2^64), a non-shared buffer,
/// `isLockFree` and `pause`. One difference from node v26.13.1 (V8): `isLockFree(4.5)` is
/// true here, as `sec-atomics.islockfree` step 1 says (ToIntegerOrInfinity gives 4); V8 gives
/// false.
#[test]
fn each_function_gives_the_spec_values() {
    let src = r#"
const sab = new SharedArrayBuffer(16);
const i8 = new Int8Array(sab), u16 = new Uint16Array(sab), i32 = new Int32Array(sab);
const u32 = new Uint32Array(sab), b64 = new BigInt64Array(sab), bu64 = new BigUint64Array(sab);
print(Atomics.store(i8, 0, 300), Atomics.load(i8, 0), Atomics.add(i8, 0, 100), Atomics.load(i8, 0));
print(Atomics.sub(u16, 1, 1), Atomics.load(u16, 1), Atomics.add(u16, 1, 2), Atomics.load(u16, 1));
print(Atomics.store(i32, 1, 12), Atomics.and(i32, 1, 6), Atomics.or(i32, 1, 9), Atomics.xor(i32, 1, 3),
  Atomics.load(i32, 1), Atomics.exchange(i32, 1, -5), Atomics.load(u32, 1));
print(Object.is(Atomics.store(i32, 0, -0), 0), Atomics.store(i32, 0, 3.7), Atomics.store(i32, 0, Infinity),
  Atomics.load(i32, 0), Atomics.store(i32, 0, "-2.9"), Atomics.load(i32, 0));
print(Atomics.compareExchange(i32, 2, 1, 7), Atomics.load(i32, 2), Atomics.compareExchange(i32, 2, 0, 9),
  Atomics.load(i32, 2), Atomics.compareExchange(i8, 8, 9 + 256, 1), Atomics.load(i8, 8));
print(Atomics.exchange(b64, 1, -1n), Atomics.load(b64, 1), Atomics.load(bu64, 1), Atomics.add(b64, 1, 2n),
  Atomics.load(b64, 1), Atomics.store(b64, 1, 2n ** 64n + 5n), Atomics.load(b64, 1),
  Atomics.compareExchange(bu64, 1, 5n, 2n ** 64n - 1n), Atomics.load(b64, 1));
const plain = new Int16Array(4);
print(Atomics.add(plain, 3, 5), Atomics.sub(plain, 3, 7), Atomics.load(plain, 3), Atomics.xor(plain, 3, -1), plain[3]);
print([1, 2, 3, 4, 5, 8, 0, -1, "4", 4.5, NaN].map((n) => Atomics.isLockFree(n)).join());
print(Atomics.pause(), Atomics.pause(1), Atomics.pause(-0), Atomics.pause(-5), Atomics.pause(2 ** 60));
for (const bad of [1.5, "1", null, NaN, Infinity, 1n, {}, true]) {
  try { Atomics.pause(bad); print("pause", String(bad), "no throw"); } catch (e) { print("pause", typeof bad, e.constructor.name); }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "300 44 44 -112",
            "0 65535 65535 1",
            "12 12 4 13 14 14 4294967291",
            "true 3 Infinity 0 -2 -2",
            "0 0 0 9 9 1",
            "1 -1 18446744073709551615 -1 1 18446744073709551621 5 5 -1",
            "0 5 -2 -2 1",
            // node v26.13.1: 4.5 gives false (see the doc).
            "true,true,false,true,false,true,false,false,true,true,false",
            "undefined undefined undefined undefined undefined",
            "pause number TypeError",
            "pause string TypeError",
            "pause object TypeError",
            "pause number TypeError",
            "pause number TypeError",
            "pause bigint TypeError",
            "pause object TypeError",
            "pause boolean TypeError",
        ])
    );
}

/// The errors and their order: the type checks before any conversion, ToIndex and the length
/// check, the conversion order (index, then each value, then the timeout or count), a detach
/// or a shrink in a `valueOf` (RevalidateAtomicAccess), `wait` and `notify` on a non-shared
/// buffer, and the results of `notify` and of `wait` with timeout 0. One difference from
/// node v26.13.1 (V8): a length-tracking view whose buffer shrinks below the element in a
/// `valueOf` is a RangeError here, as RevalidateAtomicAccess step 5 says (byteIndexInBuffer ≥
/// the buffer's byte length; the view is not out of bounds); V8 throws a TypeError.
#[test]
fn errors_and_conversion_order_follow_the_spec() {
    let src = r#"
const err = (f) => { try { return "ok " + String(f()); } catch (e) { return e.constructor.name; } };
const sab = new SharedArrayBuffer(16);
const i32 = new Int32Array(sab), b64 = new BigInt64Array(sab);
print(err(() => Atomics.load(new Float64Array(sab), 0)), err(() => Atomics.load(new Uint8ClampedArray(sab), 0)),
  err(() => Atomics.load(new Float16Array(sab), 0)), err(() => Atomics.load({}, 0)), err(() => Atomics.load([1], 0)),
  err(() => Atomics.load(i32, 4)), err(() => Atomics.load(i32, -1)), err(() => Atomics.load(i32, 2 ** 53)),
  err(() => Atomics.load(i32, "1")), err(() => Atomics.load(i32)));
print(err(() => Atomics.wait(new Int16Array(sab), 0, 0, 0)), err(() => Atomics.wait(new Uint32Array(sab), 0, 0, 0)),
  err(() => Atomics.wait(new Int32Array(4), 0, 0, 0)), err(() => Atomics.notify(new Int16Array(sab), 0)),
  err(() => Atomics.notify(new Int32Array(4), 9)), err(() => Atomics.waitAsync(new BigUint64Array(sab), 0, 0n)),
  err(() => Atomics.add(b64, 0, 1)), err(() => Atomics.add(i32, 0, 1n)));
print(Atomics.notify(new Int32Array(4), 0), Atomics.notify(new BigInt64Array(8), 0, 2), Atomics.notify(i32, 0),
  Atomics.notify(i32, 0, -3), Atomics.notify(b64, 1, Infinity));
print(Atomics.wait(i32, 0, 1, 0), Atomics.wait(i32, 0, 0, 0), Atomics.wait(b64, 1, 0n, 0), Atomics.wait(b64, 1, 1n, 0),
  Atomics.wait(i32, 0, 0, -Infinity), Atomics.wait(i32, 0, 2 ** 32, 0), Atomics.wait(b64, 1, 2n ** 64n, 0));
const log = [];
const idx = (i) => ({ valueOf() { log.push("index"); return i; } });
const val = (v, n) => ({ valueOf() { log.push(n); return v; } });
Atomics.compareExchange(i32, idx(0), val(0, "expected"), val(1, "replacement"));
Atomics.store(i32, idx(0), val(1, "value"));
Atomics.wait(i32, idx(0), val(5, "value"), val(0, "timeout"));
Atomics.notify(i32, idx(0), val(1, "count"));
print(log.join());
log.length = 0;
print(err(() => Atomics.load(i32, idx(9))), err(() => Atomics.store(new Float32Array(4), idx(0), val(0, "v"))), log.join());
const ab = new ArrayBuffer(8);
const t = new Int32Array(ab);
print(err(() => Atomics.store(t, 0, { valueOf() { ab.transfer(); return 1; } })), ab.detached,
  err(() => Atomics.load(t, 0)));
const rab = new ArrayBuffer(8, { maxByteLength: 16 });
const fixed = new Int32Array(rab, 0, 2), tracking = new Int32Array(rab);
print(err(() => Atomics.add(fixed, 1, { valueOf() { rab.resize(4); return 1; } })), rab.byteLength);
rab.resize(8);
print(err(() => Atomics.add(tracking, 1, { valueOf() { rab.resize(4); return 1; } })), rab.byteLength);
rab.resize(8);
const ab2 = new ArrayBuffer(8);
print(err(() => Atomics.notify(new Int32Array(ab2), 0, { valueOf() { ab2.transfer(); return 1; } })));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "TypeError TypeError TypeError TypeError TypeError RangeError RangeError RangeError ok 0 ok 0",
            "TypeError TypeError TypeError TypeError RangeError TypeError TypeError TypeError",
            "0 0 0 0 0",
            "not-equal timed-out timed-out not-equal timed-out timed-out timed-out",
            "index,expected,replacement,index,value,index,value,timeout,index,count",
            "RangeError TypeError index",
            "TypeError true TypeError",
            "TypeError 4",
            // node v26.13.1: "TypeError 4" (see the doc).
            "RangeError 4",
            "ok 0",
        ])
    );
}

/// `waitAsync` on one isolate: the result object (`async`, then `value`, both {w, e, c}, an
/// ordinary object), "not-equal" and "timed-out" (timeout 0) with `async` false, a promise
/// otherwise; `notify` counts the async waiters in FIFO order and their promises resolve
/// with "ok" after the script, in notify order.
#[test]
fn wait_async_resolves_in_notify_order() {
    let src = r#"
const i32 = new Int32Array(new SharedArrayBuffer(16));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); return (x.writable ? "w" : "-") + (x.enumerable ? "e" : "-") + (x.configurable ? "c" : "-"); };
const r1 = Atomics.waitAsync(i32, 0, 42);
print(r1.async, r1.value, Reflect.ownKeys(r1).join(), Object.getPrototypeOf(r1) === Object.prototype, d(r1, "async"), d(r1, "value"));
const r2 = Atomics.waitAsync(i32, 0, 0, 0);
print(r2.async, r2.value);
const r3 = Atomics.waitAsync(i32, 0, 0);
const r4 = Atomics.waitAsync(i32, 0, 0, 1e9);
const r5 = Atomics.waitAsync(i32, 1, 0);
print(r3.async, r3.value instanceof Promise, Object.getPrototypeOf(r3.value) === Promise.prototype);
r3.value.then((v) => print("r3", v));
r4.value.then((v) => print("r4", v));
r5.value.then((v) => print("r5", v));
print(Atomics.notify(i32, 0, 1), Atomics.notify(i32, 0), Atomics.notify(i32, 0), Atomics.notify(i32, 1, 0));
Promise.resolve().then(() => print("job"));
print(Atomics.notify(i32, 1));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "false not-equal async,value true wec wec",
            "false timed-out",
            "true true true",
            "1 1 0 0",
            "1",
            "job",
            "r3 ok",
            "r4 ok",
            "r5 ok",
        ])
    );
}

// ---- several isolates and the host loop (Rust-driven) ------------------------------------------

thread_local! {
    /// What `say` printed on this thread.
    static SAID: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `say(...args)`: as `print`, into [`SAID`] of this thread.
fn say<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    SAID.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// The lines `say` printed on this thread, taken.
fn said() -> Vec<String> {
    SAID.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

/// An isolate with `ep9_builtins::TABLE`, `print` and `say`.
fn agent(stress: bool) -> Isolate {
    let mut iso = common::isolate(stress);
    iso.register_host_function("say", 1, say);
    iso
}

/// Runs the sloppy script `src` on `iso`, then a microtask checkpoint.
fn script(iso: &mut Isolate, src: &str) -> Result<(), Exception> {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the script compiles");
    iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
}

/// The block of the global SharedArrayBuffer `sab` of `iso`.
fn block_of(iso: &mut Isolate) -> SharedBlockRef {
    iso.with_cx(|cx| {
        let sab = cx.global_binding("sab")?.expect("a global sab");
        cx.shared_array_buffer_block(sab)
    })
    .expect("a SharedArrayBuffer")
}

/// Makes the global `sab` of `iso`: a SharedArrayBuffer on `block`.
fn adopt(iso: &mut Isolate, block: &SharedBlockRef) {
    iso.with_cx(|cx| {
        let proto = cx.intrinsic(IntrinsicId::SharedArrayBufferPrototype);
        let sab = cx.new_shared_array_buffer_on_block(proto, block)?;
        let global = cx.intrinsic(IntrinsicId::GlobalObject);
        cx.set(global, "sab", sab, true).map(|_| ())
    })
    .expect("the SharedArrayBuffer is made");
}

/// The host loop: block for the async waiters of `iso`, then a checkpoint, until it has none
/// (the deadline: a test bug, not a time bound of the test).
fn settle(iso: &mut Isolate) {
    let end = Instant::now() + DEADLINE;
    while iso.async_waiter_count() > 0 {
        assert!(Instant::now() < end, "the async waiters never ended");
        iso.block_for_async_waiters(Duration::from_millis(5));
        iso.perform_microtask_checkpoint().expect("no exception");
    }
}

/// A deadline for each wait of one thread for another (not a time bound of the test).
const DEADLINE: Duration = Duration::from_secs(120);

/// `Atomics.wait` blocks an isolate on another thread until a notify of this isolate on the
/// same block ("ok"); a second waiter on another index times out ("timed-out"), and
/// "not-equal" does not block. Both waiters are BigInt64 and Int32 views of one block. Plain
/// and under GC stress.
#[test]
fn wait_blocks_until_a_notify_from_another_isolate() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut main = agent(stress);
            script(&mut main, "var sab = new SharedArrayBuffer(16);").unwrap();
            let block = block_of(&mut main);
            let other = std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn(move || {
                    let mut iso = agent(stress);
                    adopt(&mut iso, &block);
                    script(
                        &mut iso,
                        "const i32 = new Int32Array(sab), b64 = new BigInt64Array(sab);
                         say(Atomics.wait(i32, 1, 1, 0), Atomics.wait(i32, 1, 0, 1));
                         say(Atomics.wait(b64, 1, 0n));",
                    )
                    .unwrap();
                    said()
                })
                .unwrap();
            // Spin until the waiter is in the list (a notify of 1 then wakes it).
            script(
                &mut main,
                "const b64 = new BigInt64Array(sab);
                 let n = 0;
                 while ((n = Atomics.notify(b64, 1, 1)) === 0) {}
                 say(n, Atomics.notify(b64, 1));",
            )
            .unwrap();
            assert_eq!(other.join().unwrap(), ["not-equal timed-out", "ok"]);
            assert_eq!(said(), ["1 0"]);
        });
    }
}

/// A termination (`TerminateHandle`, the runner's watchdog) ends an `Atomics.wait` with no
/// timeout: the script ends with the termination.
#[test]
fn a_termination_ends_a_wait() {
    common::on_big_stack(|| {
        let mut main = agent(false);
        script(&mut main, "var sab = new SharedArrayBuffer(8);").unwrap();
        let block = block_of(&mut main);
        let (to_main, handle) = std::sync::mpsc::channel();
        let other = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let mut iso = agent(false);
                adopt(&mut iso, &block);
                to_main.send(iso.terminate_handle()).unwrap();
                script(
                    &mut iso,
                    "const i32 = new Int32Array(sab); Atomics.store(i32, 1, 1); Atomics.wait(i32, 0, 0);",
                )
            })
            .unwrap();
        let handle = handle.recv_timeout(DEADLINE).unwrap();
        // Wait until the other isolate is about to wait (a short sleep makes it likely that the
        // wait itself sees the termination; either way the run must end terminated).
        script(
            &mut main,
            "const i32 = new Int32Array(sab); while (Atomics.load(i32, 1) === 0) {}",
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        handle.terminate();
        let r = other.join().unwrap();
        assert!(
            r.is_err_and(|e| e.terminated),
            "the wait ended with the termination"
        );
    });
}

/// `[[CanBlock]]` false (`Isolate::set_can_block`): `Atomics.wait` is a TypeError after the
/// conversions (DoWait step 10), even when the value is not equal; `waitAsync` still works.
#[test]
fn can_block_false_makes_wait_a_type_error() {
    common::on_big_stack(|| {
        let mut iso = agent(false);
        iso.set_can_block(false);
        script(
            &mut iso,
            "const i32 = new Int32Array(new SharedArrayBuffer(8));
             const log = [];
             for (const v of [0, 1]) {
               try { Atomics.wait(i32, 0, { valueOf() { log.push('value'); return v; } }, 0); }
               catch (e) { say(e.constructor.name, log.join()); }
             }
             say(Atomics.waitAsync(i32, 0, 1).value);
             try { Atomics.wait(new Int16Array(8), 0, 0, 0); } catch (e) { say(e.constructor.name); }",
        )
        .unwrap();
        assert_eq!(
            said(),
            [
                "TypeError value",
                "TypeError value,value",
                "not-equal",
                "TypeError"
            ]
        );
        iso.set_can_block(true);
        script(
            &mut iso,
            "say(Atomics.wait(new Int32Array(new SharedArrayBuffer(8)), 0, 0, 0));",
        )
        .unwrap();
        assert_eq!(said(), ["timed-out"]);
    });
}

/// The `atomicsHelper.js` timer form: a `waitAsync` timeout resolves while the microtask
/// checkpoint spins on `Promise.then` (no host loop), and a host loop
/// (`Isolate::block_for_async_waiters`, then a checkpoint) resolves an idle one. A GC between
/// the `waitAsync` and its end keeps the promise (GC stress). Not node's output: node resolves
/// a `waitAsync` timeout in an event-loop task, which a microtask spin never reaches.
#[test]
fn wait_async_timeouts_resolve_in_a_spin_and_in_a_host_loop() {
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut iso = agent(stress);
            script(
                &mut iso,
                "const i32 = new Int32Array(new SharedArrayBuffer(8));
                 let done = false;
                 Atomics.waitAsync(i32, 0, 0, 5).value.then((v) => { say('spin', v); done = true; });
                 const start = Date.now();
                 (function spin() { if (!done && Date.now() - start < 100000) Promise.resolve().then(spin); })();",
            )
            .unwrap();
            assert_eq!(said(), ["spin timed-out"]);
            assert_eq!(iso.async_waiter_count(), 0);
            script(
                &mut iso,
                "Atomics.waitAsync(i32, 1, 0, 1).value.then((v) => say('idle', v));
                 Atomics.waitAsync(i32, 1, 0).value.then((v) => say('notified', v));
                 Atomics.waitAsync(i32, 1, 0, 1e9).value.then((v) => say('never', v));",
            )
            .unwrap();
            iso.collect_garbage();
            // The 1 ms one ends in the host loop (or already in the script's checkpoint, under
            // GC stress); then a notify of 1 takes the first of the two left (FIFO), and the
            // last stays.
            let end = Instant::now() + DEADLINE;
            while iso.async_waiter_count() > 2 {
                assert!(Instant::now() < end, "the timeout never came");
                iso.block_for_async_waiters(Duration::from_millis(5));
                iso.perform_microtask_checkpoint().unwrap();
            }
            script(&mut iso, "say(Atomics.notify(i32, 1, 1));").unwrap();
            assert_eq!(iso.async_waiter_count(), 1);
            assert_eq!(said(), ["idle timed-out", "1", "notified ok"]);
        });
    }
}

/// A notify from an isolate on another thread resolves the `waitAsync` promise of this one
/// through its inbox and the host loop (the other side spins until the waiter is in the list).
#[test]
fn wait_async_is_notified_from_another_isolate() {
    common::on_big_stack(|| {
        let mut main = agent(false);
        script(
            &mut main,
            "var sab = new SharedArrayBuffer(8);
             Atomics.waitAsync(new Int32Array(sab), 0, 0).value.then((v) => say('main', v));",
        )
        .unwrap();
        let block = block_of(&mut main);
        let other = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let mut iso = agent(false);
                adopt(&mut iso, &block);
                script(
                    &mut iso,
                    "while (Atomics.notify(new Int32Array(sab), 0) === 0) {} say('woke');",
                )
                .unwrap();
                said()
            })
            .unwrap();
        settle(&mut main);
        assert_eq!(other.join().unwrap(), ["woke"]);
        assert_eq!(said(), ["main ok"]);
    });
}

/// An isolate that ends with async waiters takes them out of the list: a later `notify` on
/// the block counts 0 (the agents of a test262 run end this way).
#[test]
fn the_drop_of_an_isolate_removes_its_async_waiters() {
    common::on_big_stack(|| {
        let mut main = agent(false);
        script(&mut main, "var sab = new SharedArrayBuffer(8);").unwrap();
        let block = block_of(&mut main);
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let mut iso = agent(false);
                adopt(&mut iso, &block);
                script(&mut iso, "Atomics.waitAsync(new Int32Array(sab), 0, 0); Atomics.waitAsync(new Int32Array(sab), 0, 0);").unwrap();
                assert_eq!(iso.async_waiter_count(), 2);
            })
            .unwrap()
            .join()
            .unwrap();
        script(&mut main, "say(Atomics.notify(new Int32Array(sab), 0));").unwrap();
        assert_eq!(said(), ["0"]);
    });
}

// ---- the lazy group (TA-D14; the TA-3 form of `tests/typed_array.rs`) --------------------------

/// The live cells after the last collection (out of line: the `GcStats` copy stays in this
/// frame, `common::conservative_roots`).
#[inline(never)]
fn live_cells(i: &Isolate) -> usize {
    i.gc_stats().last_marked_cells
}

/// The live cells after the sloppy script `src` (no GC stress) and a scrubbed full collection;
/// `rust` runs first, in the same isolate. Verifies the heap.
fn cells_after(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    common::on_big_stack(move || {
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("the script compiles");
        let mut iso = common::isolate(false);
        iso.with_cx(rust).expect("no exception");
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
            .expect("no exception");
        common::scrub_and_collect(&mut iso);
        let live = live_cells(&iso);
        black_box(&mut iso);
        iso.verify_heap().expect("a valid heap");
        live
    })
}

/// [`cells_after`], the smallest of 3 runs (a stale stack word does not count).
fn min_cells(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    (0..3)
        .map(|_| cells_after(src, rust))
        .min()
        .expect("3 runs")
}

fn nothing(_cx: &mut Cx<'_>) -> JsResult<()> {
    Ok(())
}

/// Realm init makes no `%Atomics%`: a script that reads `Atomics` once leaves more live cells
/// than one that reads nothing, by at least one cell per definition and intrinsic of the six
/// modules of the group (`array_buffer`, `data_view`, `typed_array`, `typed_array_proto`,
/// `base64`, `atomics`), and the read installs the whole group, the same cells as an
/// `ArrayBuffer` read. `Atomics` with the other keys installs the group once.
/// `Cx::has_intrinsic(Atomics)` answers true and installs nothing; `Cx::intrinsic(Atomics)`
/// installs the group.
#[test]
fn the_atomics_key_joins_the_lazy_group() {
    let size: usize = [
        ep9_builtins::array_buffer::MODULE,
        ep9_builtins::data_view::MODULE,
        ep9_builtins::typed_array::MODULE,
        ep9_builtins::typed_array::proto::MODULE,
        ep9_builtins::typed_array::base64::MODULE,
        ep9_builtins::atomics::MODULE,
    ]
    .iter()
    .map(|m: &BuiltinModule| m.defs.len() + m.intrinsics.len())
    .sum();
    let base = min_cells("", nothing);
    let group = min_cells("Atomics;", nothing).saturating_sub(base);
    assert!(
        group >= size,
        "the group adds {group} cells, fewer than its {size} definitions and intrinsics"
    );
    assert_eq!(
        min_cells("ArrayBuffer;", nothing).saturating_sub(base),
        group,
        "an ArrayBuffer read installs the same group"
    );
    // The installs of the group that a run made, rounded.
    let installs = |live: usize| (live.saturating_sub(base) + group / 2) / group;
    for src in [
        "Atomics; Int8Array; ArrayBuffer;",
        "typeof Atomics;",
        "Atomics.load;",
    ] {
        assert_eq!(installs(min_cells(src, nothing)), 1, "{src}");
    }
    fn has(cx: &mut Cx<'_>) -> JsResult<()> {
        assert!(cx.has_intrinsic(IntrinsicId::Atomics));
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", has)),
        0,
        "has_intrinsic installs nothing"
    );
    fn get(cx: &mut Cx<'_>) -> JsResult<()> {
        cx.intrinsic(IntrinsicId::Atomics);
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", get)),
        1,
        "Cx::intrinsic installs the group"
    );
}

/// A termination (`TerminateHandle`, from another thread) ends
/// `Isolate::block_for_async_waiters` before its `max` (TA-4 JIT review F2): the host block
/// checks the terminate word at least every `waiters::SLICE`, as a sync `Atomics.wait` does,
/// and leaves the termination pending. It returns true, and a next call returns true at once
/// (TA-4 JIT review r1 F1: the host stops its loop, it does not spin). The `max` of 20 s and
/// the checks below bound the test: a regress fails it, it does not hang.
#[test]
fn a_termination_ends_the_block_for_async_waiters() {
    common::on_big_stack(|| {
        let mut iso = agent(false);
        script(
            &mut iso,
            "Atomics.waitAsync(new Int32Array(new SharedArrayBuffer(4)), 0, 0);",
        )
        .unwrap();
        assert_eq!(iso.async_waiter_count(), 1);
        let handle = iso.terminate_handle();
        let watchdog = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            handle.terminate();
        });
        let start = Instant::now();
        let terminated = iso.block_for_async_waiters(Duration::from_secs(20));
        let took = start.elapsed();
        watchdog.join().unwrap();
        assert!(
            took < Duration::from_secs(10),
            "the block ran {took:?}: the termination did not end it"
        );
        assert!(terminated, "the block reports the pending termination");
        let again = Instant::now();
        assert!(iso.block_for_async_waiters(Duration::from_secs(20)));
        assert!(
            again.elapsed() < Duration::from_secs(10),
            "a terminated block returns at once"
        );
        assert_eq!(iso.async_waiter_count(), 1, "the block resolves nothing");
    });
}
