//! `%TypedArray%.from` and `%TypedArray%.of` at the JS level (TypedArray TA-3 TA-CTORS R3;
//! `src/typed_array/from_of.rs`; ECMA-262 `sec-%typedarray%.from`, `sec-%typedarray%.of`,
//! `sec-typedarraycreatefromconstructor`, `sec-validatetypedarray`). Each script prints the
//! lines of node v26.13 for the same script (`print` here, `console.log` there) and runs with
//! and without GC stress.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `%TypedArray%.from` (`sec-%typedarray%.from`) reads the whole iterator before it constructs
/// its this value (IteratorToList, then TypedArrayCreateFromConstructor with «len»), then maps
/// each value with `thisArg` and `k`. The this value must be a constructor and the mapper
/// callable, both checked before `@@iterator` is read; `of` checks its this value too. An
/// error from `@@iterator`, `next`, a `value` getter or the mapper ends the call; a
/// `@@iterator` that is not callable, and an undefined or null source, are a TypeError.
#[test]
fn the_iterator_is_read_before_the_create_and_errors_stop_it() {
    let src = r#"
// 1. The whole iterator is read before the constructor runs; the mapper runs after it.
const log = [];
class Logged extends Int8Array {
  constructor(n) { log.push("ctor " + n); super(n); }
}
const it = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { log.push("next " + i); return i < 3 ? { value: i++ * 2, done: false } : { done: true }; } };
  }
};
const r = Logged.from(it, function (v, k) { log.push("map " + v + " " + k + " " + (this === undefined ? "u" : this.tag)); return v + 1; }, { tag: "t" });
print(log.join(","));
print(r.constructor.name, r.length, Array.prototype.join.call(r));
// 2. A this value that is no constructor, a mapper that is no function: TypeError, and
//    @@iterator is not read.
let reads = 0;
const watched = { get [Symbol.iterator]() { reads++; return undefined; }, length: 0 };
for (const f of [() => Int8Array.from.call({}, watched), () => Int8Array.from.call(() => {}, watched),
  () => Int8Array.from(watched, {}), () => Int8Array.of.call(Math.max, 1)]) {
  try { f(); print("no error"); } catch (e) { print(e.constructor.name); }
}
print("reads", reads);
// 3. An iterator that throws: in @@iterator, in next, in the value getter; a @@iterator that
//    is not callable; a source that is undefined or null.
const thrower = (where) => ({
  [Symbol.iterator]() {
    if (where === "iterator") throw new RangeError("iterator");
    return { next() {
      if (where === "next") throw new RangeError("next");
      return { get value() { throw new RangeError("value"); }, done: false };
    } };
  }
});
for (const w of ["iterator", "next", "value"]) {
  try { Float64Array.from(thrower(w)); } catch (e) { print(e.constructor.name, e.message); }
}
for (const s of [{ [Symbol.iterator]: 1 }, undefined, null]) {
  try { Int8Array.from(s); } catch (e) { print(e.constructor.name); }
}
// 4. A mapper that throws stops the copy.
try { Int8Array.from([1, 2], () => { throw new SyntaxError("map"); }); } catch (e) { print(e.constructor.name, e.message); }
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "next 0,next 1,next 2,next 3,ctor 3,map 0 0 t,map 2 1 t,map 4 2 t",
            "Logged 3 1,3,5",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "reads 0",
            "RangeError iterator",
            "RangeError next",
            "RangeError value",
            "TypeError",
            "TypeError",
            "TypeError",
            "SyntaxError map",
        ])
    );
}

/// The array-like path of `from` (LengthOfArrayLike, then the create, then Get per index) and
/// `of`: holes, a length that is a string or negative, a primitive source (a string is
/// iterable, a number is not), the element conversions (Uint8Clamped, Float16, BigInt, a
/// content type mismatch), the mapper with `thisArg`. TypedArrayCreateFromConstructor keeps a
/// longer result of a custom this, and gives a TypeError for a shorter one, a non-TypedArray
/// and an out-of-bounds one; `of` and `from` on a subclass make the subclass.
#[test]
fn the_array_like_path_and_a_custom_this_follow_the_spec() {
    let src = r#"
const j = (t) => Array.prototype.join.call(t);
// 1. Array-likes: holes read as undefined (NaN), length conversion, a getter order, a
//    primitive source (a string is iterable, a number is not).
const log = [];
const al = { get length() { log.push("length"); return "3"; }, get 0() { log.push("0"); return 7; }, 2: 9 };
class L extends Float32Array { constructor(n) { log.push("ctor " + n); super(n); } }
print(j(L.from(al)), log.join(","));
print(j(Int16Array.from("123")), j(Int16Array.from(5)), j(Uint8Array.from({ length: -1 })));
print(j(Uint8ClampedArray.from([300, -5, 1.5, 2.5])), j(Float16Array.from([1.337, 65520])));
print(j(BigInt64Array.from([1n, -1n], (v) => v * 2n)), j(BigUint64Array.of(-1n)));
try { BigInt64Array.from([1]); } catch (e) { print(e.constructor.name); }
try { Int8Array.of(1n); } catch (e) { print(e.constructor.name); }
// 2. The array-like path maps with this and k; LengthOfArrayLike errors propagate.
print(j(Int8Array.from({ length: 2, 0: 5, 1: 6 }, function (v, k) { return v * 10 + k + this.add; }, { add: 1 })));
try { Int8Array.from({ get length() { throw new EvalError("len"); } }); } catch (e) { print(e.constructor.name, e.message); }
// 3. A custom this: a longer TypedArray is kept; a shorter one, a non-TypedArray and an
//    out-of-bounds one are a TypeError; another content type fails only in Set.
const big = new Int16Array(5);
print(Int8Array.from.call(function () { return big; }, [1, 2]) === big, j(big));
for (const make of [() => new Int8Array(1), () => ({ length: 9 }), () => {
  const b = new ArrayBuffer(4, { maxByteLength: 8 }); const t = new Int8Array(b, 2, 2); b.resize(1); return t; }]) {
  try { Int8Array.of.call(function () { return make(); }, 1, 2); print("no error"); } catch (e) { print(e.constructor.name); }
}
try { Int8Array.from.call(function () { return new BigInt64Array(2); }, [1, 2]); } catch (e) { print(e.constructor.name); }
print(j(Int8Array.from.call(function () { return new BigInt64Array(2); }, [])));
// 4. of on a subclass, with its own prototype and constructor argument.
class Sub extends Uint16Array {
  constructor(...a) { print("Sub", a.length, a[0]); super(...a); }
  sum() { return Array.prototype.reduce.call(this, (a, b) => a + b, 0); }
}
const s = Sub.of(1, 2, 3);
print(s instanceof Sub, Object.getPrototypeOf(s) === Sub.prototype, s.sum(), s.length);
const f = Sub.from([4, 5]);
print(f instanceof Sub, f.sum());
print(Int8Array.of().length, Int8Array.from([]).length, Int8Array.of.length, Int8Array.from.length);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "7,NaN,9 length,ctor 3,0",
            "1,2,3  ",
            "255,0,2,2 1.3369140625,Infinity",
            "2,-2 18446744073709551615",
            "TypeError",
            "TypeError",
            "51,62",
            "EvalError len",
            "true 1,2,0,0,0",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "0,0",
            "Sub 1 3",
            "true true 6 3",
            "Sub 1 2",
            "true 9",
            "0 0 0 1",
        ])
    );
}

/// Each `Set` is the element `[[Set]]` of the new TypedArray, so a mapper or a `valueOf` that
/// detaches, shrinks or grows its buffer makes the later writes go to the new length (an index
/// out of bounds is ignored; `from` and `of` still return the target). A target that is
/// detached at the create is a TypeError before any value is converted. The values of an
/// iterable are read before the first `Set`. A length-tracking target on a growable
/// SharedArrayBuffer takes the values.
#[test]
fn a_mapper_or_value_of_that_resizes_the_target_is_seen() {
    let src = r#"
const j = (t) => Array.prototype.join.call(t);
// 1. A mapper that detaches the target: the later Sets are ignored, from returns the target.
const ab = new ArrayBuffer(4);
const target = new Int8Array(ab);
const r = Int8Array.from.call(function () { return target; }, [1, 2, 3], (v) => {
  if (v === 2) ab.transfer();
  return v * 10;
});
print(r === target, r.length, ab.detached);
// 2. A mapper that shrinks and then grows a resizable target (array-like path).
const rb = new ArrayBuffer(3, { maxByteLength: 4 });
const rt = new Int8Array(rb);
const rr = Int8Array.from.call(function () { return rt; }, { length: 3, 0: 1, 1: 2, 2: 3 }, (v, k) => {
  if (k === 0) rb.resize(1);
  if (k === 1) rb.resize(4);
  return v + 10;
});
print(rr === rt, j(rt));
// 3. A valueOf in of shrinks and grows the target (the test262 form).
const ob = new ArrayBuffer(3, { maxByteLength: 4 });
const ot = new Int8Array(ob);
const one = { valueOf() { ob.resize(0); return 1; } };
const two = { valueOf() { ob.resize(4); return 2; } };
print(Int8Array.of.call(function () { return ot; }, one, two, 3) === ot, j(ot));
// 4. A this that returns a detached target: TypeError before any Set.
let calls = 0;
try {
  Int8Array.of.call(function () { const t = new Int8Array(4); t.buffer.transfer(); return t; },
    { valueOf() { calls++; return 1; } });
} catch (e) { print(e.constructor.name, calls); }
// 5. The iterated values are all read before the Sets: a valueOf that empties the source
//    array changes nothing.
const values = [0, { valueOf() { values.length = 0; return 100; } }, 2];
print(j(Int32Array.from(values)));
// 6. A length-tracking target on a growable SharedArrayBuffer.
const sab = new SharedArrayBuffer(2, { maxByteLength: 8 });
const st = new Int16Array(sab);
print(j(Int16Array.from.call(function (n) { sab.grow(n * 2); return st; }, [7, 8, 9])), st.length);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "true 0 true",
            "true 11,12,13,0",
            "true 0,2,3,0",
            "TypeError 0",
            "0,100,2",
            "7,8,9 3",
        ])
    );
}

fn compile_sloppy(src: &str) -> ep9_bytecode::VerifiedUnit {
    ep9_frontend::compile(
        src,
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles")
}

/// The `tests/iterator.rs` form: runs `setup`, then a loop that a termination stops at its
/// first poll (a poll sets the interrupt budget to at least 10,000 units), then requests
/// termination again and runs `call` in a third script, on a thread with a 60 s wait (it only
/// stops a hung test). True if the call ended terminated and the isolate kept working.
fn terminates_in(setup: &'static str, call: &'static str) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut iso = common::isolate(false);
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(compile_sloppy(setup), "setup.js").map(|_| ())
            })
            .expect("the setup runs");
            iso.terminate_handle().terminate();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(compile_sloppy("for (;;) {}"), "fill.js")
                    .map(|_| ())
            });
            assert!(
                r.is_err_and(|e| e.terminated),
                "the budget fill loop ends terminated"
            );
            iso.terminate_handle().terminate();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(
                    compile_sloppy(&format!(
                        "var stage = 'before'; var r = {call}; stage = 'reached';"
                    )),
                    "call.js",
                )
                .map(|_| ())
            });
            let terminated = r.is_err_and(|e| e.terminated);
            let stage = common::global_text(&mut iso, "stage");
            // The isolate keeps working.
            iso.with_cx_checkpoint(|cx| {
                cx.run_script(compile_sloppy("stage = 'again'"), "again.js")
                    .map(|_| ())
            })
            .expect("runs");
            let again = common::global_text(&mut iso, "stage");
            let _ = tx.send((terminated, stage, again));
        })
        .expect("a thread");
    let got = rx.recv_timeout(std::time::Duration::from_secs(60));
    got == Ok((true, Some("before".to_string()), Some("again".to_string())))
}

/// The fill loops over a list (`from` step 6.e and InitializeTypedArrayFromList in the
/// constructor, after IteratorToList; `of` steps 5-6 over its arguments) take one interrupt
/// budget unit per element (`Cx::poll_interrupt`; H23 "Interrupt poll"; Codex TA-3 r1): a
/// Number element runs no JS, so without the poll a termination waits for the whole fill.
/// `from` and the constructor take 6,001 units to read the iterator (one per step) and 6,000
/// more to fill; `of` fills 12,000. Each is more than the 10,128 units that the budget holds
/// at most, so only a poll in the fill loop stops it.
#[test]
fn termination_stops_the_list_fill_loops() {
    const ARRAY: &str = "var a = []; for (var i = 0; i < 6000; i++) a.push(i);";
    const BIG: &str = "var b = []; for (var i = 0; i < 12000; i++) b.push(i);";
    let not_stopped = [
        (ARRAY, "Int8Array.from(a)"),
        (ARRAY, "new Int8Array(a)"),
        (BIG, "Int8Array.of(...b)"),
    ]
    .into_iter()
    .filter(|&(setup, call)| !terminates_in(setup, call))
    .map(|(_, call)| call)
    .collect::<Vec<_>>();
    assert_eq!(
        not_stopped,
        Vec::<&str>::new(),
        "calls that a termination did not stop"
    );
}
