//! String.prototype.normalize (ECMA-262 §22.1.3.15) over `ep9_intl::normalize` (INTL W4
//! NORMALIZE), with and without GC stress. The expected lines are node v26.13.1's output for the
//! same script (`print` = `console.log` of the arguments, each through `String`, joined by a
//! space). `hex` prints the code units, so a lone surrogate shows as it is.

mod common;
use common::{lines, run_both};

const NORMALIZE: &str = r#"const hex = s => Array.from({length: s.length}, (_, i) => s.charCodeAt(i).toString(16)).join(".");
const P = String.prototype.normalize;
const d = Object.getOwnPropertyDescriptor(String.prototype, "normalize");
print(typeof P, P.length, P.name, d.writable, d.enumerable, d.configurable, "prototype" in P);
const s = "ẛ̣";
print(hex(s.normalize()), hex(s.normalize("NFC")), hex(s.normalize("NFD")), hex(s.normalize("NFKC")), hex(s.normalize("NFKD")));
const t = "Å⫝̸क़Ω̈́";
print(hex(t.normalize("NFC")), hex(t.normalize("NFD")), hex(t.normalize("NFKD")));
print(hex("ﷺ".normalize("NFKC")), hex("각".normalize("NFD")), hex("각".normalize("NFC")));
print(hex("á\uD800é\uDC00́".normalize()), hex("\uDFFFÅ\uDBFF".normalize("NFKC")), hex("𝅗𝅥".normalize("NFC")));
print(hex("á\uD800é\uDC00".normalize("NFD")), hex("😀Å".normalize("NFC")));
const abc = "abc";
print(abc.normalize() === abc, "á".normalize("NFC") === "á", "".normalize("NFKD") === "");
print(String.prototype.normalize.call(1e21, "NFC"), String.prototype.normalize.call(true), String.prototype.normalize.call({toString() { return "á"; }}) === "á");
print("abc".normalize({toString() { return "NFKC"; }}), "abc".normalize(undefined), "N" + "FD" === "NFD" && "á".normalize("N" + "FD").length);
let a = "", b = "";
for (let i = 0; i < 200; i++) { a += "Ā"; b += "Ā"; }
print(a.normalize("NFD") === b, b.normalize("NFC") === a, b.normalize("NFKC") === a);
const long = "a" + "̣́".repeat(1000);
const n = long.normalize("NFD");
print(n.length, hex(n.slice(0, 4)), hex(n.slice(1001, 1003)), long.normalize("NFC").length);
const r = [];
function t2(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t2(() => "a".normalize("NFE"));
t2(() => "a".normalize("nfc"));
t2(() => "a".normalize(null));
t2(() => "a".normalize(""));
t2(() => "a".normalize(Symbol()));
t2(() => P.call(null));
t2(() => P.call(undefined, "NFE"));
t2(() => P.call(Symbol()));
t2(() => new P());
print(r.join(" "));
const log = [];
try { P.call({toString() { log.push("this"); return "x"; }}, {toString() { log.push("form"); return "NFX"; }}); } catch (e) { log.push(e.constructor.name); }
try { P.call({toString() { log.push("this"); throw new SyntaxError(); }}, {toString() { log.push("form"); return "NFC"; }}); } catch (e) { log.push(e.constructor.name); }
print(log.join(" "));
"#;

#[test]
fn normalize_gives_the_node_lines() {
    assert_eq!(
        run_both(NORMALIZE),
        lines(&[
            "function 0 normalize true false true false",
            "1e9b.323 1e9b.323 17f.323.307 1e69 73.323.307",
            "c5.2add.338.915.93c.3a9.308.301 41.30a.2add.338.915.93c.3a9.308.301 41.30a.2add.338.915.93c.3a9.308.301",
            "635.644.649.20.627.644.644.647.20.639.644.64a.647.20.648.633.644.645 1100.1161.11a8 ac01",
            "e1.d800.e9.dc00.301 dfff.c5.dbff d834.dd57.d834.dd65",
            "61.301.d800.65.301.dc00 d83d.de00.c5",
            "true true true",
            "1e+21 true true",
            "abc abc 2",
            "true true true",
            "2001 61.323.323.323 301.301 2000",
            "RangeError RangeError RangeError RangeError TypeError TypeError TypeError TypeError TypeError",
            "this form RangeError this SyntaxError",
        ])
    );
}
