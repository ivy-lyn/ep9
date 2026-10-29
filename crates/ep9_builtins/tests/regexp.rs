//! RegExp (§22.2, M4-P3 REGEXP1): the constructor, `exec`, `test`, `toString`, the flag
//! accessors, `[Symbol.match]`, `[Symbol.replace]`, `[Symbol.search]`, `[Symbol.split]`, and
//! `String.prototype.match` / `search`. M5-P2 REGEXP2: `[Symbol.matchAll]`,
//! `%RegExpStringIteratorPrototype%`, `String.prototype.matchAll`, `RegExp.escape`, Annex B
//! `RegExp.prototype.compile` and the legacy static accessors (`RegExp.$1` ...).
//!
//! The expected lines are node v24.22.1's output for the same script (`print` = the
//! arguments' ToString joined by spaces), except where a test says that the spec (the
//! legacy RegExp features proposal, which test262 `legacy-regexp` tests) and V8 differ. `common::run_both` runs each script with and without
//! GC stress (a collection before every allocation, then the heap verifier).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The plan §5.4 acceptance line (the e2e case `ep9 -e`).
#[test]
fn acceptance_typeof_and_test() {
    assert_eq!(
        common::run_both(r#"print(typeof /a+/g, /a+/g.test("aa"))"#),
        lines(&[r#"object true"#])
    );
}

/// RegExp ( pattern, flags ) §22.2.4.1: called and constructed, from a RegExp, from a RegExp-like object, the same object back.
#[test]
fn constructor_forms() {
    assert_eq!(
        common::run_both(
            r#"var re = /x/gi;
print(RegExp("a", "g").flags, new RegExp(re).source, new RegExp(re).flags, new RegExp(re, "y").flags);
print(RegExp(re) === re, new RegExp(re) === re, RegExp(re, undefined) === re, RegExp(re, "g") === re);
var like = { source: "b+", flags: "m", constructor: RegExp };
like[Symbol.match] = true;
print(RegExp(like) === like, new RegExp(like).source, new RegExp(like).flags);
print(RegExp().source, new RegExp(undefined, undefined).flags === "", RegExp("a/b").source, String(RegExp("\n")));
print(RegExp.length, RegExp.name, RegExp.prototype.constructor === RegExp, RegExp[Symbol.species] === RegExp);
print(Object.getPrototypeOf(RegExp("a")) === RegExp.prototype, RegExp("a").lastIndex);"#
        ),
        lines(&[
            r#"g x gi y"#,
            r#"true false true false"#,
            r#"true b+ m"#,
            r#"(?:) true a\/b /\n/"#,
            r#"2 RegExp true true"#,
            r#"true 0"#
        ])
    );
}

/// Bad flags and bad patterns are SyntaxErrors; a symbol pattern is a TypeError.
#[test]
fn constructor_errors() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
print(name(function () { RegExp("a", "gg"); }), name(function () { RegExp("a", "x"); }), name(function () { RegExp("a", "uv"); }));
print(name(function () { RegExp("("); }), name(function () { new RegExp("[b-a]"); }), name(function () { RegExp(Symbol()); }));"#
        ),
        lines(&[
            r#"SyntaxError SyntaxError SyntaxError"#,
            r#"SyntaxError SyntaxError TypeError"#
        ])
    );
}

/// A subclass gets its prototype from new.target (RegExpAlloc), and its own exec runs through RegExpExec.
#[test]
fn subclass_and_new_target() {
    assert_eq!(
        common::run_both(
            r#"class R extends RegExp {
  exec(s) { this.calls = (this.calls || 0) + 1; return super.exec(s); }
}
var r = new R("b", "g");
print(r instanceof R, r instanceof RegExp, r.test("abc"), r.calls, r.lastIndex);
print("abcb".replace(r, "X"), r.calls, "abcb".match(r).length);"#
        ),
        lines(&[r#"true true true 1 2"#, r#"aXcX 4 2"#])
    );
}

/// RegExpBuiltinExec §22.2.7.2: index, input, groups, captures that did not take part, the indices array (d).
#[test]
fn exec_result() {
    assert_eq!(
        common::run_both(
            r#"var m = /(a)(x)?(c)/.exec("zabc".replace("b", ""));
print(m.length, m.index, m.input, m[0], m[1], m[2], m[3], m.groups, Object.keys(m).join());
var d = /(?<l>a)(?<n>x)?/d.exec("_a");
print(JSON.stringify(d.indices), JSON.stringify(d.indices.groups), Object.getPrototypeOf(d.groups), Object.keys(d).join());
var g = /(?<year>\d{4})-(?<month>\d{2})/.exec("on 2024-05!");
print(g.groups.year, g.groups.month, Object.getPrototypeOf(g.groups) === null, Object.keys(g.groups).join());
print(/(?<a>x)|(?<a>y)/.exec("y").groups.a, JSON.stringify(/(?<a>x)|(?<a>y)/d.exec("x").indices.groups));
print(/x/.exec("y"), /\u{1F600}/u.exec("a\u{1F600}").index, /(?<ab>.)/.exec("q").groups.ab);"#
        ),
        lines(&[
            r#"4 1 zac ac a undefined c undefined 0,1,2,3,index,input,groups"#,
            r#"[[1,2],[1,2],null] {"l":[1,2]} null 0,1,2,index,input,groups,indices"#,
            r#"2024 05 true year,month"#,
            r#"y {"a":[0,1]}"#,
            r#"null 1 q"#
        ])
    );
}

/// lastIndex: read with ToLength, kept for a non-global RegExp, set past each match (g), only at lastIndex (y), reset to 0 after a failure.
#[test]
fn last_index_global_and_sticky() {
    assert_eq!(
        common::run_both(
            r#"var g = /a/g, s = "aXaXa", out = [];
while (g.exec(s)) out.push(g.lastIndex);
print(out.join(), g.lastIndex);
var y = /a/y;
print(y.test("ba"), y.lastIndex, (y.lastIndex = 1, y.test("ba")), y.lastIndex, y.test("ba"), y.lastIndex);
var n = /a/; n.lastIndex = 3; print(n.test("a"), n.lastIndex);
var big = /a/g; big.lastIndex = 9; print(big.exec("aaa"), big.lastIndex);
var calls = 0, v = /b/g; v.lastIndex = { valueOf: function () { calls++; return 1; } };
print(v.exec("abc").index, calls, v.lastIndex);
var f = /a/g; Object.defineProperty(f, "lastIndex", { writable: false });
try { f.exec("a"); print("no error"); } catch (e) { print(e.name); }
var nf = /a/; Object.defineProperty(nf, "lastIndex", { writable: false }); print(nf.exec("a")[0]);"#
        ),
        lines(&[
            r#"1,3,5 0"#,
            r#"false 0 true 2 false 0"#,
            r#"true 3"#,
            r#"null 0"#,
            r#"1 1 2"#,
            r#"TypeError"#,
            r#"a"#
        ])
    );
}

/// The flag accessors, flags (it reads each accessor; ep9 reads unicodeSets before sticky as §22.2.6.4 orders them, V8 reads it last, so the fake has no unicodeSets) and source (EscapeRegExpPattern); %RegExp.prototype% itself; other receivers throw.
#[test]
fn flags_and_source_accessors() {
    assert_eq!(
        common::run_both(
            r#"var r = /a/dgimsuy;
print(r.flags, r.hasIndices, r.global, r.ignoreCase, r.multiline, r.dotAll, r.unicode, r.unicodeSets, r.sticky);
print(/[a]/v.unicodeSets, /a/v.flags, /x/.flags === "");
print(RegExp.prototype.source, RegExp.prototype.global, RegExp.prototype.flags === "", String(RegExp.prototype));
var seen = [];
var fake = {};
["hasIndices", "global", "ignoreCase", "multiline", "dotAll", "unicode", "sticky"].forEach(function (k) {
  Object.defineProperty(fake, k, { get: function () { seen.push(k); return k.length > 6; } });
});
var flagsGetter = Object.getOwnPropertyDescriptor(RegExp.prototype, "flags").get;
print(flagsGetter.call(fake), seen.join());
var names = [];
["global", "source", "sticky"].forEach(function (k) {
  var get = Object.getOwnPropertyDescriptor(RegExp.prototype, k).get;
  try { get.call({}); } catch (e) { names.push(e.name); }
  try { get.call(1); } catch (e) { names.push(e.name); }
});
print(names.join(), new RegExp("/").source, RegExp("[/]").source, RegExp("").source, String(/a\/b/gi));
print(RegExp.prototype.toString.call({ source: "S", flags: "F" }), Object.prototype.toString.call(/x/));"#
        ),
        lines(&[
            r#"dgimsuy true true true true true true false true"#,
            r#"true v true"#,
            r#"(?:) undefined true /(?:)/"#,
            r#"dimu hasIndices,global,ignoreCase,multiline,dotAll,unicode,sticky"#,
            r#"TypeError,TypeError,TypeError,TypeError,TypeError,TypeError \/ [/] (?:) /a\/b/gi"#,
            r#"/S/F [object RegExp]"#
        ])
    );
}

/// RegExp.prototype[@@match] §22.2.6.8 and String.prototype.match §22.1.3.13.
#[test]
fn symbol_match() {
    assert_eq!(
        common::run_both(
            r#"print("abcabc".match(/b/).index, "abcabc".match(/b/g).join(), "xyz".match(/b/g), "a.c".match(".").index);
print("\u{1F4A9}\u{1F4A9}".match(/(?:)/gu).length, "\u{1F4A9}".match(/(?:)/g).length, "aaa".match(/a*?/g).join("|"));
var r = /a/g; r.lastIndex = 2; print("aaa".match(r).length, r.lastIndex);
print("ab".match({ [Symbol.match]: function (s) { return "custom " + s; } }), "1+1".match("1+1"));
print(String.prototype.match.length, String.prototype.match.name);"#
        ),
        lines(&[
            r#"1 b,b null 0"#,
            r#"3 3 |||"#,
            r#"3 0"#,
            r#"custom ab null"#,
            r#"1 match"#
        ])
    );
}

/// RegExp.prototype[@@replace] §22.2.6.11 with GetSubstitution §22.1.3.19.1: $ patterns, captures, named captures, a function, empty matches.
#[test]
fn symbol_replace() {
    assert_eq!(
        common::run_both(
            r#"print("abc".replace(/b/, "[$&|$`|$'|$$|$0|$1]"), "abc".replace(/(b)/, "[$1|$01|$10|$2|$11]"));
var ten = /(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)/;
print("abcdefghij".replace(ten, "$10-$11-$1$2"), "2024-05".replace(/(?<y>\d+)-(?<m>\d+)/, "$<m>/$<y>|$<zz>|$<"));
print("abc".replace(/x*/g, "-"), "aaa".replace(/a/g, "b"), "aXbX".replace(/X/g, function (m, i) { return i; }));
print("2024-05".replace(/(?<y>\d+)-(?<m>\d+)/, function () { return JSON.stringify([].slice.call(arguments)); }));
print("abc".replace(/(z)?b/, function (m, z, i, s) { return [m, z, i, s].join(","); }));
print("\u{1F4A9}".replace(/(?:)/gu, "-"), "\u{1F4A9}".replace(/(?:)/g, "-").length, "aaa".replace(/a/y, "b"));
var r = /a/g; r.lastIndex = 2; print("aa".replace(r, "b"), r.lastIndex);
print("abc".replace("b", "$&$&"), "x".replace(/x/, "$"), "x".replace(/x/, "$<a>"));"#
        ),
        lines(&[
            r#"a[b|a|c|$|$0|$1]c a[b|b|b0|$2|b1]c"#,
            r#"j-a1-ab 05/2024||$<"#,
            r#"-a-b-c- bbb a1b3"#,
            r#"["2024-05","2024","05",0,"2024-05",{"y":"2024","m":"05"}]"#,
            r#"ab,,1,abcc"#,
            r#"-💩- 5 baa"#,
            r#"bb 0"#,
            r#"abbc $ $<a>"#
        ])
    );
}

/// RegExpExec with an own exec: @@replace reads each result through Get (length, 0, index, captures, groups) in spec order.
#[test]
fn replace_with_custom_exec_results() {
    assert_eq!(
        common::run_both(
            r#"var log = [];
var re = /./g;
var results = [
  { length: 2, 0: "bc", 1: "X", index: 1, groups: { n: "N" } },
  null
];
re.exec = function () { var r = results.shift(); log.push("exec"); return r; };
print("abcd".replace(re, "<$1$<n>$&>"), log.join());
var pos = /./;
pos.exec = function () { return { length: 1, 0: "q", index: -5 }; };
print("zz".replace(pos, "!"));
var bad = /./;
bad.exec = function () { return 1; };
try { "a".replace(bad, ""); } catch (e) { print(e.name); }"#
        ),
        lines(&[r#"a<XNbc>d exec,exec"#, r#"!z"#, r#"TypeError"#])
    );
}

/// RegExp.prototype[@@search] §22.2.6.12 and String.prototype.search §22.1.3.22: lastIndex is saved and restored.
#[test]
fn symbol_search() {
    assert_eq!(
        common::run_both(
            r#"var r = /b/g; r.lastIndex = 2;
print("abc".search(r), r.lastIndex, "abc".search(/z/), "a.b".search("."), "a.b".search(/\./));
print("ab".search({ [Symbol.search]: function () { return 42; } }), String.prototype.search.name);"#
        ),
        lines(&[r#"1 2 -1 0 1"#, r#"42 search"#])
    );
}

/// RegExp.prototype[@@split] §22.2.6.14: limit, captures, empty matches, Unicode, a sticky RegExp.
#[test]
fn symbol_split() {
    assert_eq!(
        common::run_both(
            r#"print(JSON.stringify("a-b-c".split(/-/)), JSON.stringify("a-b-c".split(/-/, 2)), JSON.stringify("a-b".split(/(-)/)));
print(JSON.stringify("abc".split(/(?:)/)), JSON.stringify("".split(/x/)), JSON.stringify("".split(/(?:)/)));
print(JSON.stringify("\u{1F4A9}a".split(/(?:)/u).length), JSON.stringify("\u{1F4A9}a".split(/(?:)/).length));
print(JSON.stringify("a1b22c".split(/(\d)(x)?/)), JSON.stringify("test".split(/(?:)/, 0)), JSON.stringify("a b  c".split(/\s+/)));
print(JSON.stringify("ab".split(/a*?/)), JSON.stringify("ab".split(/a*/)), JSON.stringify("axbxc".split(/x/y)));
print(JSON.stringify("A<B>bold</B>and<CODE>coded</CODE>".split(/<(\/)?([^<>]+)>/)));"#
        ),
        lines(&[
            r#"["a","b","c"] ["a","b"] ["a","-","b"]"#,
            r#"["a","b","c"] [""] []"#,
            r#"2 3"#,
            r#"["a","1",null,"b","2",null,"","2",null,"c"] [] ["a","b","c"]"#,
            r#"["a","b"] ["","b"] ["a","b","c"]"#,
            r#"["A",null,"B","bold","/","B","and",null,"CODE","coded","/","CODE",""]"#
        ])
    );
}

/// The @@split fast path (one search per piece) gives what the spec loop gives: a species constructor and an own exec force the spec loop.
#[test]
fn split_slow_path_equals_fast_path() {
    assert_eq!(
        common::run_both(
            r#"var inputs = ["a, b,c ,  d", "", "xx", "\u{1F4A9},\u{1F4A9}", ";;a;", "no separators here"];
var pats = [/\s*,\s*/, /(,)/, /x*/, /(?:)/u, /;/y, /(?=a)/];
function viaSpecies(re, s) {
  class S extends RegExp {}
  var c = new S(re.source, re.flags);
  return s.split(c);
}
var same = 0, all = 0;
for (var i = 0; i < inputs.length; i++) for (var j = 0; j < pats.length; j++) {
  all++;
  var fast = JSON.stringify(inputs[i].split(pats[j]));
  var slow = JSON.stringify(viaSpecies(pats[j], inputs[i]));
  if (fast === slow) same++; else print("diff", i, j, fast, slow);
}
var saved = RegExp.prototype.exec;
RegExp.prototype.exec = function (s) { return saved.call(this, s); };
var viaExec = JSON.stringify("a, b,c".split(/\s*,\s*/));
RegExp.prototype.exec = saved;
print(same, all, viaExec);"#
        ),
        lines(&[r#"36 36 ["a","b","c"]"#])
    );
}

/// @@split constructs the splitter with SpeciesConstructor(rx, %RegExp%) and the flags + "y".
#[test]
fn species_constructor_in_split() {
    assert_eq!(
        common::run_both(
            r#"var seen;
class Spy extends RegExp {
  static get [Symbol.species]() { return function (p, f) { seen = [p.source, f]; return new RegExp(p, f); }; }
}
print(JSON.stringify("a,b".split(new Spy(",", "gi"))), JSON.stringify(seen));
var r = /,/; r.constructor = undefined; print(JSON.stringify("a,b".split(r)));
var bad = /,/; bad.constructor = 1;
try { "a,b".split(bad); } catch (e) { print(e.name); }"#
        ),
        lines(&[r#"["a","b"] [",","giy"]"#, r#"["a","b"]"#, r#"TypeError"#])
    );
}

/// IsRegExp §7.2.8 reads @@match first; a RegExp object without one still counts (startsWith, includes, replaceAll).
#[test]
fn is_regexp_checks() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
print(name(function () { "a".startsWith(/a/); }), name(function () { "a".includes(/a/); }), name(function () { "a".replaceAll(/a/, ""); }));
var r = /a/; r[Symbol.match] = false;
print("/a/".startsWith(r), "aa".replaceAll(/a/g, "b"));
var plain = /a/; delete RegExp.prototype[Symbol.match];
print(name(function () { "a".endsWith(plain); }));"#
        ),
        lines(&[
            r#"TypeError TypeError TypeError"#,
            r#"true bb"#,
            r#"TypeError"#
        ])
    );
}

/// Forms regexp.js uses (v8-v7 kernel): split on a cookie string, replace with a function, test, exec loops.
#[test]
fn regexp_kernel_shapes() {
    assert_eq!(
        common::run_both(
            r#"var s = "ZFPhygher=VC=74.125.75.1&VCPhygher=ra-HF&CersreerqPhygher=ra-HF; __hgzn=1; __hgzo=2";
print(s.split(/\s?;\s?/).length, s.split(/[+, ]/).length, s.replace(/=/g, ":").length);
print(s.replace(/(\w+)=(\w*)/g, function (all, k, v) { return v + "=" + k; }).slice(0, 30));
var re1 = /(((\w+):\/\/)([^\/:]*)(:(\d+))?)?([^#?]*)(\?([^#]*))?(#(.*))?/;
var m = re1.exec("http://example.com:80/path?q=1#frag");
print(m.length, m[3], m[4], m[6], m[7], m[9], m[11]);
var count = 0, g = /\d+/g, t = "a1b22c333";
while (g.exec(t)) count++;
print(count, /^\s*|\s*$/g.test("  x  "), "  x  ".replace(/^\s*|\s*$/g, "").length);
var big = ""; for (var i = 0; i < 2000; i++) big += "w" + i + " ";
print(big.split(/\s+/).length, big.replace(/w/g, "").length, big.match(/\d+/g).length);"#
        ),
        lines(&[
            r#"3 3 83"#,
            r#"VC=ZFPhygher=74.125.75.1&ra=VC"#,
            r#"12 http example.com 80 /path q=1 frag"#,
            r#"3 true 1"#,
            r#"2001 8890 2000"#
        ])
    );
}

/// Many RegExp objects and programs while GC runs (each run_both pass also runs with a collection before every allocation).
#[test]
fn many_regexps_under_gc() {
    assert_eq!(
        common::run_both(
            r#"var ok = 0;
for (var i = 0; i < 60; i++) {
  var r = new RegExp("(?<n>a" + i + ")b", i % 2 ? "g" : "");
  var m = r.exec("xa" + i + "b");
  if (m && m.groups.n === "a" + i && m.index === 1) ok++;
  if ("a1b,a2b".replace(/a(\d)b/g, "$1") === "1,2") ok++;
}
print(ok);"#
        ),
        lines(&[r#"120"#])
    );
}

/// RegExp.prototype[@@matchAll] (§22.2.6.9), %RegExpStringIteratorPrototype% (§22.2.9) and
/// String.prototype.matchAll (§22.1.3.14): the iterator protocol, empty matches (code points
/// in Unicode mode), a non-global matcher, the copied lastIndex, the receiver errors.
#[test]
fn match_all_and_the_string_iterator() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
var it = "a1b22c333".matchAll(/\d+/g);
print(Object.prototype.toString.call(it), it[Symbol.toStringTag], it[Symbol.iterator]() === it);
var out = []; for (var m of it) out.push(m[0] + "@" + m.index); print(out.join());
var r = it.next(); print(r.value, r.done, Object.keys(r).join());
print([..."aXbX".matchAll("X")].length, [..."".matchAll(/(?:)/g)].length, [..."ab".matchAll(/(?:)/g)].map(function (m) { return m.index; }).join());
print(name(function () { "a".matchAll(/a/); }), name(function () { RegExp.prototype[Symbol.matchAll].call(1, "a"); }), name(function () { it.next.call({}); }));
var once = /a/[Symbol.matchAll]("aab"); var o1 = once.next(), o2 = once.next(); print(o1.value[0], o1.done, o2.value, o2.done);
print([..."\u{1F600}x".matchAll(/(?:)/gu)].map(function (m) { return m.index; }).join(), [..."\u{1F600}x".matchAll(/(?:)/g)].map(function (m) { return m.index; }).join());
print(String.prototype.matchAll.length, RegExp.prototype[Symbol.matchAll].name, it.next.name, it.next.length, String.prototype.matchAll.name);
var re = /(?<d>\d)/g; re.lastIndex = 2; var it2 = re[Symbol.matchAll]("1234"); print(re.lastIndex, [...it2].map(function (m) { return m.groups.d; }).join(), re.lastIndex);
var called = 0; var fake = { [Symbol.matchAll]: function (s) { called++; return s; } }; print("str".matchAll(fake), called);
print(name(function () { "a".matchAll({ [Symbol.match]: true, flags: undefined }); }));"#
        ),
        lines(&[
            r#"[object RegExp String Iterator] RegExp String Iterator true"#,
            r#"1@1,22@3,333@6"#,
            r#"undefined true value,done"#,
            r#"2 1 0,1,2"#,
            r#"TypeError TypeError TypeError"#,
            r#"a false undefined true"#,
            r#"0,2,3 0,1,2,3"#,
            r#"1 [Symbol.matchAll] next 0 matchAll"#,
            r#"2 3,4 2"#,
            r#"str 1"#,
            r#"TypeError"#
        ])
    );
}

/// RegExp.escape ( S ) (§22.2.5.1): a leading letter or digit, syntax characters, the other
/// punctuators, white space, line terminators and lone surrogates; the TypeError of a
/// non-string.
#[test]
fn regexp_escape() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
print(RegExp.escape("foo.bar"), RegExp.escape("1+1"), RegExp.escape("_a"), RegExp.escape("^$\\.*+?()[]{}|/"));
print(JSON.stringify(RegExp.escape(" \t\n\v\f\r,-=<>#&!%:;@~'`\"")));
print(JSON.stringify(RegExp.escape("   ﻿　𐀀x\udfffé")));
print(name(function () { RegExp.escape(1); }), name(function () { RegExp.escape(new String("a")); }), RegExp.escape.length, RegExp.escape.name);
print(new RegExp(RegExp.escape("a.b*c")).test("a.b*c"), new RegExp(RegExp.escape("a.b*c")).test("aXbbc"), RegExp.escape(""), RegExp.escape("\u{1F600}") === "\u{1F600}");
var d = Object.getOwnPropertyDescriptor(RegExp, "escape"); print(d.writable, d.enumerable, d.configurable);"#
        ),
        lines(&[
            r#"\x66oo\.bar \x31\+1 _a \^\$\\\.\*\+\?\(\)\[\]\{\}\|\/"#,
            r#""\\x20\\t\\n\\v\\f\\r\\x2c\\x2d\\x3d\\x3c\\x3e\\x23\\x26\\x21\\x25\\x3a\\x3b\\x40\\x7e\\x27\\x60\\x22""#,
            r#""\\xa0\\u2028\\u2029\\ufeff\\u3000𐀀x\\udfffé""#,
            r#"TypeError TypeError 1 escape"#,
            r#"true false  true"#,
            r#"true false true"#
        ])
    );
}

/// Annex B RegExp.prototype.compile (B.2.4.1): the new source, flags and program on the same
/// object, lastIndex 0, a RegExp pattern, the SyntaxErrors (the object keeps its source), the
/// ToString order, a literal. Spec, not node: `compile` on a subclass instance is a TypeError
/// ([[LegacyFeaturesEnabled]] false; node v24 compiles it).
#[test]
fn annex_b_compile() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
var re = /a/g; re.lastIndex = 3; var r2 = re.compile("b+", "i");
print(r2 === re, re.source, re.flags, re.lastIndex, re.test("BB"), re.global);
re.compile(/x/m); print(re.source, re.flags);
print(name(function () { re.compile(/x/, "g"); }), name(function () { re.compile("("); }), name(function () { re.compile("a", "gg"); }), re.source, re.flags);
class R extends RegExp {}
print(name(function () { new R("a").compile("b"); }), name(function () { RegExp.prototype.compile.call({}, "a"); }), RegExp.prototype.compile.length);
re.compile(); print(re.source, re.flags === "");
var lit = function () { return /q/; }; var l = lit(); l.compile("w", "y"); print(l.source, l.sticky, lit().source);
var frozen = /a/; Object.defineProperty(frozen, "lastIndex", { writable: false }); print(name(function () { frozen.compile("b"); }), frozen.source);
var calls = []; var p = { toString: function () { calls.push("p"); return "c"; } }; var f = { toString: function () { calls.push("f"); return "g"; } }; re.compile(p, f); print(calls.join(), re.source, re.flags);"#
        ),
        lines(&[
            r#"true b+ i 0 true false"#,
            r#"x m"#,
            r#"TypeError SyntaxError SyntaxError x m"#,
            r#"TypeError TypeError 2"#,
            r#"(?:) true"#,
            r#"w true q"#,
            r#"TypeError b"#,
            r#"p,f c g"#
        ])
    );
}

/// The legacy static accessors of %RegExp% (Annex B, legacy RegExp features): the values after
/// exec, replace, split and match, the initial empty strings, `RegExp.input =`, a failed exec
/// changes nothing, the property order and attributes. Spec, not node: a receiver other than
/// %RegExp% is a TypeError, and `$1`-`$9` have no setter (node v24 has both).
#[test]
fn annex_b_legacy_statics() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
print(JSON.stringify([RegExp.input, RegExp.lastMatch, RegExp.$1]));
/(\w)(\d)?/.exec("--a1--");
print(RegExp.$1, RegExp.$2, RegExp.lastMatch, RegExp["$&"], RegExp.lastParen, RegExp["$+"], RegExp.leftContext, RegExp["$`"], RegExp.rightContext, RegExp["$'"], RegExp.input, RegExp.$_, RegExp.$3 === "");
"xyz".replace(/y/, ""); print(RegExp.lastMatch, RegExp.leftContext, RegExp.rightContext, JSON.stringify(RegExp.$1));
/(a)|(b)/.exec("b"); print(JSON.stringify([RegExp.$1, RegExp.$2, RegExp.lastParen]));
RegExp.input = 42; print(RegExp.input, typeof RegExp.$_, RegExp.lastMatch);
/q/.exec("nope"); print(RegExp.lastMatch);
print(name(function () { Object.getOwnPropertyDescriptor(RegExp, "$1").get.call({}); }), name(function () { Object.getOwnPropertyDescriptor(RegExp, "input").set.call({}, "a"); }));
var d = Object.getOwnPropertyDescriptor(RegExp, "input"); print(d.enumerable, d.configurable, typeof d.set, typeof d.get);
var d1 = Object.getOwnPropertyDescriptor(RegExp, "$1"); print(typeof d1.set, d1.enumerable, d1.configurable);
print(Object.getOwnPropertyNames(RegExp).join());
"a-b-c".split(/-/); print(RegExp.lastMatch, RegExp.leftContext);
"abcabc".match(/b/g); print(RegExp.lastMatch, RegExp.leftContext);"#
        ),
        lines(&[
            r#"["","",""]"#,
            r#"a 1 a1 a1 1 1 -- -- -- -- --a1-- --a1-- true"#,
            r#"y x z """#,
            r#"["","b","b"]"#,
            r#"42 string b"#,
            r#"b"#,
            r#"TypeError TypeError"#,
            r#"false true function function"#,
            r#"undefined false true"#,
            r#"length,name,prototype,input,$_,lastMatch,$&,lastParen,$+,leftContext,$`,rightContext,$',$1,$2,$3,$4,$5,$6,$7,$8,$9,escape"#,
            r#"- a-b"#,
            r#"b abca"#
        ])
    );
}

/// A match of a RegExp subclass instance invalidates the legacy values (InvalidateLegacy-
/// RegExpStaticProperties: each getter throws) until a %RegExp% match. Spec, not node (node v24
/// keeps the values).
#[test]
fn annex_b_legacy_statics_after_a_subclass_exec() {
    assert_eq!(
        common::run_both(
            r#"function name(f) { try { f(); return "none"; } catch (e) { return e.name; } }
/(z)/.exec("z"); class R extends RegExp {} new R("b").exec("abc");
print(name(function () { return RegExp.lastMatch; }), name(function () { return RegExp.input; }), name(function () { return RegExp.$1; }));
/c/.exec("abc"); print(RegExp.lastMatch, RegExp.input);"#
        ),
        lines(&[r#"TypeError TypeError TypeError"#, r#"c abc"#])
    );
}

/// @@replace step 15 in list order when results are replaced at once (M5-P2, F5): a template
/// with captures, named groups and `$\``; a replace function; a getter that gives a user
/// `exec` for the second match only (record, object, record: the third waits in the list);
/// 20,000 matches; `$'`, `$$` and references to missing groups.
#[test]
fn replace_in_list_order() {
    assert_eq!(
        common::run_both(
            r#"print("a1b2c3".replace(/(\d)/g, "[$1]"), "a1b2c3".replace(/(?<n>\d)/g, "{$<n>$`}"), "aaa".replace(/a*?/g, "-"));
print("a1b2c3".replace(/(?<n>\d)/g, function (m, c, i, str, g) { return "<" + m + c + i + g.n + str.length + ">"; }));
var re2 = /(\d)/g, k = 0, bexec = RegExp.prototype.exec;
Object.defineProperty(re2, "exec", { get: function () { k++; return k === 2 ? function (s) { var r = bexec.call(this, s); if (r) r[0] = "Z" + r[0]; return r; } : bexec; } });
print("a1b2c3".replace(re2, "<$1|$&>"), k);
var big = "ab".repeat(20000).replace(/a/g, "$&$&"); print(big.length, big.slice(0, 7));
print("x".replace(/x/g, "$'$`$$"), "abc".replace(/b/g, "$0$1$01"));"#
        ),
        lines(&[
            r#"a[1]b[2]c[3] a{1a}b{2a1b}c{3a1b2c} -a-a-a-"#,
            r#"a<11116>b<22326>c<33536>"#,
            r#"a<1|1>b<2|Z2><3|3> 4"#,
            r#"60000 aabaaba"#,
            r#"$ a$0$1$01c"#
        ])
    );
}
