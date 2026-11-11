//! Intl.Segmenter (ECMA-402 §19; INTL W2 SEGMENTER): the constructor and its option read
//! order, supportedLocalesOf, resolvedOptions, segment, the Segments object (containing,
//! @@iterator) and the Segment Iterator (next), with and without GC stress. The expected lines
//! are node v26.13.0's output for the same script (`print` = `console.log` of the arguments,
//! each through `String`, joined by a space).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const S = Intl.Segmenter;
print(typeof S, S.length, S.name, Object.getPrototypeOf(S) === Function.prototype);
const pd = Object.getOwnPropertyDescriptor(S, "prototype");
print(pd.writable, pd.enumerable, pd.configurable);
print(Object.getOwnPropertyNames(S).join(), Object.getOwnPropertyNames(S.prototype).join());
const seg = new S("en");
const segs = seg.segment("ab");
const SP = Object.getPrototypeOf(segs);
const it = segs[Symbol.iterator]();
const IP = Object.getPrototypeOf(it);
print(Object.getOwnPropertyNames(SP).join(), String(Object.getOwnPropertySymbols(SP).map(String)), SP.containing.length, SP[Symbol.iterator].name);
print(Object.getOwnPropertyNames(IP).join(), IP.next.length, IP[Symbol.toStringTag], Object.getPrototypeOf(IP) === Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
print(Object.prototype.toString.call(seg), Object.prototype.toString.call(segs), Object.prototype.toString.call(it));
print(JSON.stringify(seg.resolvedOptions()), JSON.stringify(new S("de-CH-u-co-phonebk", {granularity: "word"}).resolvedOptions()));
print(JSON.stringify(S.supportedLocalesOf(["de-CH", "zz", "en-u-co-phonebk"])));
class My extends S {}
print(new My("en").resolvedOptions().granularity, new My("en") instanceof S);
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => S());
t(() => new S("en", {granularity: "line"}));
t(() => new S("en", null));
t(() => new S("en", {localeMatcher: "x"}));
t(() => S.prototype.segment.call({}, "a"));
t(() => S.prototype.resolvedOptions.call(segs));
t(() => SP.containing.call(seg, 0));
t(() => SP[Symbol.iterator].call(it));
t(() => IP.next.call(segs));
t(() => seg.segment(Symbol()));
t(() => new SP.containing());
print(r.join(" "));
const reads = [];
const handler = { get(t, k) { reads.push(String(k)); return undefined; } };
new S(undefined, new Proxy({}, handler));
print(reads.join());"#;

/// The shape of %Intl.Segmenter%, its prototype, %IntlSegmentsPrototype% and
/// %IntlSegmentIteratorPrototype%, a subclass, the errors of the constructor and the
/// methods, and the order of the option reads.
#[test]
fn the_constructor_and_prototypes_give_the_node_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 Segmenter true",
            "false false false",
            "length,name,prototype,supportedLocalesOf constructor,resolvedOptions,segment",
            "containing Symbol(Symbol.iterator) 1 [Symbol.iterator]",
            "next 0 Segmenter String Iterator true",
            "[object Intl.Segmenter] [object Object] [object Segmenter String Iterator]",
            "{\"locale\":\"en\",\"granularity\":\"grapheme\"} {\"locale\":\"de-CH\",\"granularity\":\"word\"}",
            "[\"de-CH\",\"en-u-co-phonebk\"]",
            "grapheme true",
            "TypeError RangeError TypeError RangeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "localeMatcher,granularity",
        ])
    );
}

const SEGMENTS: &str = r#"const show = d => d === undefined ? "undefined" : JSON.stringify([d.segment, d.index, d.isWordLike, d.input.length]);
const all = (l, g, s) => [...new Intl.Segmenter(l, {granularity: g}).segment(s)].map(d => d.segment + (d.isWordLike === undefined ? "" : d.isWordLike ? "+" : "-")).join("|");
print(all("en", "grapheme", "e\u0301\u{1F1EF}\u{1F1F5}a\u{1F468}\u200D\u{1F469}"));
print(all("en", "word", "Hello, world 42! It's 3.14."));
print(all("en", "sentence", "Hi there. How are you? Fine."));
print(all("el", "sentence", "hello; world"));
print(all("th", "word", "ทุกสองสัปดาห์"));
print(all("ja", "word", "こんにちは世界。日本語です"));
print(JSON.stringify(all("en", "word", "")), [...new Intl.Segmenter("en", {granularity: "word"}).segment("\uD800a\uDC00")].map(d => d.segment.charCodeAt(0) + ":" + d.index + d.isWordLike).join());
const s = new Intl.Segmenter("en", {granularity: "word"}).segment("The quick (brown) fox");
print([0, 3, 4, 9, 20, 21, -1, Infinity, "5", NaN, 1.9].map(i => show(s.containing(i))).join(" "));
const it1 = s[Symbol.iterator](), it2 = s[Symbol.iterator]();
it1.next(); it1.next();
print(show(it1.next().value), show(it2.next().value), show(s.containing(10)), show(it1.next().value));
let n = 0; for (const d of s) n++;
const done = it2; while (!done.next().done) {}
print(n, JSON.stringify(done.next()), JSON.stringify(Object.keys(it1.next().value)));
const g = new Intl.Segmenter("en").segment("abc");
print(JSON.stringify(Object.keys(g.containing(1))), JSON.stringify(g.containing(1)));
const big = "word ".repeat(300);
let c = 0, last;
for (const d of new Intl.Segmenter("en", {granularity: "word"}).segment(big)) { c++; last = d.index; }
print(c, last, show(new Intl.Segmenter("en", {granularity: "word"}).segment(big).containing(1498)));"#;

/// The segments of each granularity (grapheme clusters with emoji sequences, words with
/// isWordLike, sentences, the "el" sentence tailoring, Thai and Japanese words from the
/// dictionary, lone surrogates), `containing` at the edges, two iterators of one Segments
/// object and `containing` between their steps, and a string of 1,500 code units (the
/// iterator and `containing` keep the string through GC stress).
#[test]
fn segments_and_iterators_give_the_node_lines() {
    assert_eq!(
        run_both(SEGMENTS),
        lines(&[
            "e\u{301}|\u{1f1ef}\u{1f1f5}|a|\u{1f468}\u{200d}\u{1f469}",
            "Hello+|,-| -|world+| -|42+|!-| -|It's+| -|3.14+|.-",
            "Hi there. |How are you? |Fine.",
            "hello; |world",
            "\u{e17}\u{e38}\u{e01}+|\u{e2a}\u{e2d}\u{e07}+|\u{e2a}\u{e31}\u{e1b}\u{e14}\u{e32}\u{e2b}\u{e4c}+",
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}+|\u{4e16}\u{754c}+|\u{3002}-|\u{65e5}\u{672c}\u{8a9e}+|\u{3067}\u{3059}+",
            "\"\" 55296:0false,97:1true,56320:2false",
            "[\"The\",0,true,21] [\" \",3,false,21] [\"quick\",4,true,21] [\" \",9,false,21] [\"fox\",18,true,21] undefined undefined undefined [\"quick\",4,true,21] [\"The\",0,true,21] [\"The\",0,true,21]",
            "[\"quick\",4,true,21] [\"The\",0,true,21] [\"(\",10,false,21] [\" \",9,false,21]",
            "9 {\"done\":true} [\"segment\",\"index\",\"input\",\"isWordLike\"]",
            "[\"segment\",\"index\",\"input\"] {\"segment\":\"b\",\"index\":1,\"input\":\"abc\"}",
            "600 1499 [\"word\",1495,true,1500]",
        ])
    );
}

/// The boundary table of a long string is made in one builtin call, which polls the
/// interrupt once per segment: a termination requested before the script stops the call
/// (30,000 grapheme clusters, 3 times the poll budget), and the isolate keeps working.
#[test]
fn a_termination_stops_the_boundary_table() {
    let call = format!(
        "new Intl.Segmenter().segment('{}').containing(0)",
        "a".repeat(30_000)
    );
    assert_eq!(
        common::terminate_during(&call),
        Some((true, Some("before".to_string()), Some("again".to_string())))
    );
}
