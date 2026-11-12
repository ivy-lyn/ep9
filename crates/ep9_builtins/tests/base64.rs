//! The base64 and hex methods of `Uint8Array` at the JS level (TypedArray TA-4;
//! `src/typed_array/base64.rs`; ES2026 `sec-uint8array.frombase64` and the 5 others). R1 (the
//! skeleton) pins the 6 keys with their attributes, names and lengths (only `Uint8Array` and
//! `Uint8Array.prototype` have them; `tests/typed_array.rs` pins the full key lists) and the
//! stub TypeError of each method (R5 B64 flips it). R5 appends. Each script prints the lines of
//! node v26.13.1 for the same script (`print` here, `console.log` there), except the stub
//! lines, and runs with and without GC stress.

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

/// Each method throws the stub TypeError (m8-stub, TypedArray TA-4 R1) until R5 B64 builds it.
#[test]
fn each_stub_throws_a_type_error() {
    let src = r#"
const u8 = new Uint8Array(2);
for (const [o, n] of [[Uint8Array, "fromBase64"], [Uint8Array, "fromHex"],
  [u8, "toBase64"], [u8, "setFromBase64"], [u8, "toHex"], [u8, "setFromHex"]]) {
  try {
    o[n]("AA");
    print(n, "no throw");
  } catch (e) {
    print(n, e instanceof TypeError, e.message);
  }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "fromBase64 true Uint8Array.fromBase64: not built yet (m8-stub, TypedArray TA-4)",
            "fromHex true Uint8Array.fromHex: not built yet (m8-stub, TypedArray TA-4)",
            "toBase64 true Uint8Array.prototype.toBase64: not built yet (m8-stub, TypedArray TA-4)",
            "setFromBase64 true Uint8Array.prototype.setFromBase64: not built yet (m8-stub, TypedArray TA-4)",
            "toHex true Uint8Array.prototype.toHex: not built yet (m8-stub, TypedArray TA-4)",
            "setFromHex true Uint8Array.prototype.setFromHex: not built yet (m8-stub, TypedArray TA-4)",
        ])
    );
}
