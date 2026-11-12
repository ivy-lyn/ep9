//! The base64 and hex methods of `Uint8Array` at the JS level (TypedArray TA-4;
//! `src/typed_array/base64.rs`; ES2026 `sec-uint8array.frombase64` and the 5 others). R1 (the
//! skeleton) pins the 6 keys with their attributes, names and lengths (only `Uint8Array` and
//! `Uint8Array.prototype` have them; `tests/typed_array.rs` pins the full key lists); R5 B64
//! built the 6 methods (they were stubs before R5) and pins their results. Each script
//! prints the lines of node v26.13.1 for the same script (`print` here, `console.log` there)
//! and runs with and without GC stress.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `fromBase64` and `fromHex` on `%Uint8Array%`, `toBase64`, `setFromBase64`, `toHex` and
/// `setFromHex` on `%Uint8Array.prototype%`, each {w, c} with its name and length; no other
/// TypedArray constructor or prototype has them.
#[test]
fn the_methods_have_the_spec_attributes() {
    let src = r#"
const d = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  return ("value" in x ? "d" + (x.writable ? "w" : "-") : "a") + (x.enumerable ? "e" : "-") +
    (x.configurable ? "c" : "-");
};
const row = (o, k) => k + " " + d(o, k) + " " + o[k].name + " " + o[k].length;
print(["fromBase64", "fromHex"].map((k) => row(Uint8Array, k)).join("\n"));
print(["toBase64", "setFromBase64", "toHex", "setFromHex"].map((k) => row(Uint8Array.prototype, k)).join("\n"));
print(String(Int8Array.fromBase64), String(Uint8ClampedArray.fromHex),
  String(Int8Array.prototype.toBase64), String(Uint8ClampedArray.prototype.toHex),
  String(Object.getPrototypeOf(Int8Array).fromBase64));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "fromBase64 dw-c fromBase64 1\n\
             fromHex dw-c fromHex 1",
            "toBase64 dw-c toBase64 0\n\
             setFromBase64 dw-c setFromBase64 1\n\
             toHex dw-c toHex 0\n\
             setFromHex dw-c setFromHex 1",
            "undefined undefined undefined undefined undefined",
        ])
    );
}

/// Each method on a small input (they were stubs before R5 B64): the two statics make a
/// Uint8Array, the two set methods return `{ read, written }` (an ordinary object) and write
/// the target.
#[test]
fn each_method_on_a_small_input() {
    let src = r#"
const u8 = new Uint8Array([0, 255]);
for (const [o, n, a] of [[Uint8Array, "fromBase64", "AA"], [Uint8Array, "fromHex", "AA"],
  [u8, "toBase64", undefined], [u8, "setFromBase64", "AA"], [u8, "toHex", undefined], [u8, "setFromHex", "AA"]]) {
  const r = o[n](a);
  print(n, r instanceof Uint8Array ? "Uint8Array[" + r.join(",") + "]" :
    typeof r === "object" ? JSON.stringify(r) + " " + (Object.getPrototypeOf(r) === Object.prototype) : r, u8.join(","));
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "fromBase64 Uint8Array[0] 0,255",
            "fromHex Uint8Array[170] 0,255",
            "toBase64 AP8= 0,255",
            "setFromBase64 {\"read\":2,\"written\":1} true 0,255",
            "toHex 00ff 0,255",
            "setFromHex {\"read\":2,\"written\":1} true 170,255",
        ])
    );
}

/// The options (alphabet, lastChunkHandling, omitPadding) and their read order, each
/// SyntaxError of FromBase64 and FromHex, each TypeError of a string or an option that is not
/// one, and the set methods: the bytes before an error are written, then the error.
#[test]
fn the_options_and_the_errors() {
    let src = r#"
const show = (f) => {
  try {
    const r = f();
    return r instanceof Uint8Array ? "[" + r.join(",") + "]" : typeof r === "object" ? JSON.stringify(r) : String(r);
  } catch (e) {
    return e.constructor.name;
  }
};
const bytes = new Uint8Array([251, 255, 191, 0, 1]);
print("toBase64", show(() => bytes.toBase64()), show(() => bytes.toBase64({ alphabet: "base64url" })),
  show(() => bytes.toBase64({ omitPadding: 1 })), show(() => bytes.toBase64({ alphabet: "base64url", omitPadding: true })),
  show(() => new Uint8Array(0).toBase64()), show(() => bytes.subarray(1, 2).toBase64()));
print("toHex", show(() => bytes.toHex()), show(() => new Uint8Array(0).toHex()));
print("alphabet", show(() => Uint8Array.fromBase64("-_-_")), show(() => Uint8Array.fromBase64("-_-_", { alphabet: "base64url" })),
  show(() => Uint8Array.fromBase64("+/+/", { alphabet: "base64url" })), show(() => Uint8Array.fromBase64("+/+/", { alphabet: undefined })));
for (const last of ["loose", "strict", "stop-before-partial"]) {
  print(last, ["Zm9vYg", "Zm9vYg=", "Zm9vYg==", "Zm9vYh==", "Zm9vYmE", "Zm9vY", "Zm9v", "", "  Zm 9v\nYg\t==\r\f"].map(
    (s) => show(() => Uint8Array.fromBase64(s, { lastChunkHandling: last }))).join(" "));
}
print("errors", ["Zg=a", "Z===", "Zg===", "Zm9v.", "Zm9vé", "Zm9vĀ", "=", "\v"].map(
  (s) => show(() => Uint8Array.fromBase64(s))).join(" "));
print("hex", ["", "0aFf", "0aF", "0g", "00 ", "٠٠", "ff00ff00"].map((s) => show(() => Uint8Array.fromHex(s))).join(" "));
print("types", show(() => Uint8Array.fromBase64(1)), show(() => Uint8Array.fromBase64(Object("AA"))),
  show(() => Uint8Array.fromHex(null)), show(() => Uint8Array.fromBase64("AA", null)), show(() => Uint8Array.fromBase64("AA", "x")),
  show(() => Uint8Array.fromBase64("AA", { alphabet: Object("base64") })), show(() => Uint8Array.fromBase64("AA", { alphabet: "BASE64" })),
  show(() => Uint8Array.fromBase64("AA", { lastChunkHandling: "Loose" })), show(() => bytes.toBase64(1)),
  show(() => bytes.toBase64({ alphabet: "base64\u0000" })));
const order = [];
const opts = new Proxy({}, { get(t, k) { order.push(String(k)); return undefined; } });
Uint8Array.fromBase64("AA", opts);
bytes.toBase64(opts);
new Uint8Array(1).setFromBase64("AA", opts);
print("order", order.join(","));
const t = new Uint8Array(5).fill(255);
print("set", show(() => t.setFromBase64("Zm9vYmFy")), t.join(","), show(() => t.setFromBase64("MjYyZm.9v")), t.join(","),
  show(() => t.setFromHex("0102030405060708")), t.join(","), show(() => t.setFromHex("aaag")), t.join(","),
  show(() => t.setFromHex("abc")), t.join(","), show(() => new Uint8Array(0).setFromBase64("Zm9v")),
  show(() => t.setFromBase64("Zm9vYmE", { lastChunkHandling: "stop-before-partial" })));
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "toBase64 +/+/AAE= -_-_AAE= +/+/AAE -_-_AAE  /w==",
            "toHex fbffbf0001 ",
            "alphabet SyntaxError [251,255,191] SyntaxError [251,255,191]",
            "loose [102,111,111,98] SyntaxError [102,111,111,98] [102,111,111,98] [102,111,111,98,97] SyntaxError [102,111,111] [] [102,111,111,98]",
            "strict SyntaxError SyntaxError [102,111,111,98] SyntaxError SyntaxError SyntaxError [102,111,111] [] [102,111,111,98]",
            "stop-before-partial [102,111,111] [102,111,111] [102,111,111,98] [102,111,111,98] [102,111,111] [102,111,111] [102,111,111] [] [102,111,111,98]",
            "errors SyntaxError SyntaxError SyntaxError SyntaxError SyntaxError SyntaxError SyntaxError SyntaxError",
            "hex [] [10,255] SyntaxError SyntaxError SyntaxError SyntaxError [255,0,255,0]",
            "types TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "order alphabet,lastChunkHandling,alphabet,omitPadding,alphabet,lastChunkHandling",
            "set {\"read\":4,\"written\":3} 102,111,111,255,255 SyntaxError 50,54,50,255,255 {\"read\":10,\"written\":5} 1,2,3,4,5 SyntaxError 170,2,3,4,5 SyntaxError 170,2,3,4,5 {\"read\":0,\"written\":0} {\"read\":4,\"written\":3}",
        ])
    );
}

/// A detach in an option getter, a detached receiver, a length-tracking view that shrinks
/// or grows in a getter, a view that went out of bounds, a SharedArrayBuffer target, and the
/// receiver checks (ValidateUint8Array before the string and the options).
#[test]
fn detached_resizable_and_shared_targets() {
    let src = r#"
const show = (f) => {
  try {
    const r = f();
    return r instanceof Uint8Array ? "[" + r.join(",") + "]" : typeof r === "object" ? JSON.stringify(r) : String(r);
  } catch (e) {
    return e.constructor.name;
  }
};
const getter = (k, f) => Object.defineProperty({}, k, { get() { f(); return undefined; } });
// A detach in an option getter: toBase64 and setFromBase64 read the bytes after the options.
let a = new Uint8Array([1, 2]);
print("detach", show(() => a.toBase64(getter("alphabet", () => a.buffer.transfer()))),
  show(() => (a = new Uint8Array(2), a.setFromBase64("AQI=", getter("lastChunkHandling", () => a.buffer.transfer())))),
  show(() => (a = new Uint8Array([1, 2]), a.toBase64(getter("omitPadding", () => a.buffer.transfer())))));
// A detached receiver: toHex, setFromHex and setFromBase64 throw a TypeError.
const d = new Uint8Array(2);
d.buffer.transfer();
print("detached", show(() => d.toHex()), show(() => d.setFromHex("01")), show(() => d.setFromBase64("AQI=")),
  show(() => d.toBase64()));
// A resizable buffer: a length-tracking view sees a shrink or a grow in a getter.
const rab = new ArrayBuffer(4, { maxByteLength: 8 });
const track = new Uint8Array(rab);
print("shrink", show(() => track.setFromBase64("AQIDBAUG", getter("alphabet", () => rab.resize(2)))),
  new Uint8Array(rab).join(","));
print("grow", show(() => track.setFromBase64("AQIDBAUG", getter("alphabet", () => rab.resize(8)))),
  new Uint8Array(rab).join(","), track.toHex(), track.toBase64());
const fixed = new Uint8Array(rab, 4, 2);
rab.resize(5);
print("out of bounds", show(() => fixed.toHex()), show(() => fixed.setFromHex("01")),
  show(() => new Uint8Array(rab, 2).setFromHex("0a0b0c0d")), new Uint8Array(rab).join(","));
// A SharedArrayBuffer target.
const sab = new SharedArrayBuffer(6);
const s = new Uint8Array(sab, 1, 4);
print("shared", show(() => s.setFromBase64("Zm9vYmFy")), show(() => s.setFromHex("ab")),
  new Uint8Array(sab).join(","), s.toBase64(), s.toHex());
// The receiver: only a Uint8Array.
const P = Uint8Array.prototype;
const others = [new Int8Array(1), new Uint8ClampedArray(1), new DataView(new ArrayBuffer(1)), {}, [], 1, undefined];
print("receivers", ["toBase64", "toHex", "setFromBase64", "setFromHex"].map(
  (n) => others.map((o) => show(() => P[n].call(o, "AA"))).join(",")).join(" "));
// ValidateUint8Array runs before the string check and the option getters.
let calls = 0;
const counting = getter("alphabet", () => calls++);
print("validate", show(() => P.setFromBase64.call(new Int8Array(1), 1, counting)),
  show(() => P.toBase64.call(new Int8Array(1), counting)), calls,
  show(() => new Uint8Array(1).setFromBase64(1, counting)), calls);
// The statics ignore the receiver.
print("statics", show(() => Uint8Array.fromBase64.call(Int8Array, "AQI=")), show(() => Uint8Array.fromHex.call(null, "0102")),
  Object.getPrototypeOf(Uint8Array.fromHex.call(Int16Array, "")) === Uint8Array.prototype);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "detach TypeError TypeError TypeError",
            "detached TypeError TypeError TypeError TypeError",
            "shrink {\"read\":0,\"written\":0} 0,0",
            "grow {\"read\":8,\"written\":6} 1,2,3,4,5,6,0,0 0102030405060000 AQIDBAUGAAA=",
            "out of bounds TypeError TypeError {\"read\":6,\"written\":3} 1,2,10,11,12",
            "shared {\"read\":4,\"written\":3} {\"read\":2,\"written\":1} 0,171,111,111,0,0 q29vAA== ab6f6f00",
            "receivers TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "validate TypeError TypeError 0 TypeError 0",
            "statics [1,2] [1,2] true",
        ])
    );
}
