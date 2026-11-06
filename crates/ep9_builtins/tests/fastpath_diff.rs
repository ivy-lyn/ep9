//! FASTDIFF: the fast-path differential tests (09-milestones.md:108 "every human fast path
//! matches its spec-layer path: 0 mismatches"; 10-quality.md:71, every PR from M2; plan v12
//! V12-D30 and rule 73; M7-P0 CALLHOST).
//!
//! A human fast path is hand-written code that skips spec steps under a guard, with the spec
//! steps beside it in the same build. [`INVENTORY`] lists every one at B0 (the grep
//! `fast path` over `ep9_vm` and `ep9_builtins`, the RX0b list of `regexp/symbols.rs` and the
//! JSON string slice of `json/parse.rs`), the fast paths that the same "is this still the
//! intrinsic?" test guards (`builtins::existing_function`, `Cx::is_builtin`), and the
//! M7-P0 CALLHOST paths. Each row says how its spec steps run:
//!
//! ```text
//! Off::Switch  Cx::set_human_fast_paths(false) turns it off (ep9_vm call::human_fast_paths):
//!              each case runs with the fast paths on and off, and both runs must agree
//! Off::Guard   it does not read the switch yet; each case has a second form whose guard
//!              fails (an own property, a wrapper function, a boxed value), so the spec
//!              steps run; the two forms must agree, and each form on and off too
//! ```
//!
//! A run gives the printed lines and the uncaught exception ("Name: message"). The cases of
//! the Switch rows also run under GC stress (a collection at each allocation), on and off.
//!
//! The JSON report (schema `ep9-fastdiff/v1`, the form that `cargo xtask m2-exit` item 5
//! reads): the ep9 commit and whether its tree was dirty (`commit`, `dirty`; `null` when git
//! cannot say), `fast_paths[]` with each fast path's `name`, `cases` and `mismatches`, and the
//! totals. It goes to `$EP9_FASTDIFF_JSON` if set, else to
//! `$CARGO_TARGET_TMPDIR/fastdiff.json`. `EP9_FASTDIFF_DUMP=<dir>` also writes each case
//! form as a `.js` file (with `print`), so node can run it.
//!
//! Grep hits that are not a human fast path (no spec steps beside them in the same build):
//! the profile records of `profile.rs` (they name the T0 operator fast bodies, row
//! `t0-operators`), the IC hit counters of `ic.rs` (row `ic-named`), `feedback.rs` (the
//! T0 call count, row `t0-call`), `interp/slow/mod.rs:265` (a VM or BytecodeGen bug report),
//! `fuse.rs:652` and `fuse.rs:969` (the fuse readers of a fast path: rows `regexp-flags` and
//! the H31 fuses), `realm.rs:52`, `builtins.rs:1504` and `abi/regexp.rs:18` (the identity
//! test itself), and `facts_export.rs:539` (a facts lookup, no fast path).

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local};

/// How the spec steps of a fast path run in its cases.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Off {
    /// `Cx::set_human_fast_paths(false)` turns it off.
    Switch,
    /// The second form of each case fails its guard.
    Guard,
}

/// One human fast path.
struct FastPath {
    id: &'static str,
    /// Where it is (at B0, or the M7-P0 file for a CALLHOST path).
    site: &'static str,
    off: Off,
    what: &'static str,
}

/// One case: `src`, and for a Guard row `spec`, the form whose guard fails.
struct Case {
    path: &'static str,
    name: &'static str,
    src: &'static str,
    spec: Option<&'static str>,
}

const INVENTORY: &[FastPath] = &[
    // ---- M7-P0 CALLHOST (the switch) ----
    FastPath {
        id: "host-entry",
        site: "crates/ep9_vm/src/interp/slow/call.rs host_entry",
        off: Off::Switch,
        what: "the decode-free entry of a host callee in the 9 fixed-operand call shapes",
    },
    FastPath {
        id: "forward-apply",
        site: "crates/ep9_vm/src/interp/slow/call.rs forward (apply)",
        off: Off::Switch,
        what: "f.apply(thisArg, list) of a bytecode f with an unmodified arguments object, a \
               packed Array, undefined or null: the frame pushed from the source",
    },
    FastPath {
        id: "forward-call",
        site: "crates/ep9_vm/src/interp/slow/call.rs forward (call)",
        off: Off::Switch,
        what: "f.call(thisArg, ...args) of a bytecode f: the frame pushed from the registers",
    },
    // ---- B0, the switch ----
    FastPath {
        id: "args-create",
        site: "crates/ep9_vm/src/arguments.rs:62 (create_fast, :212)",
        off: Off::Switch,
        what: "a new arguments object takes the realm's cached shape (V10-D11)",
    },
    FastPath {
        id: "clfal-length",
        site: "crates/ep9_vm/src/arguments.rs:463 (list_length)",
        off: Off::Switch,
        what: "CreateListFromArrayLike reads the length of an unmodified arguments object in \
               place",
    },
    FastPath {
        id: "clfal-elements",
        site: "crates/ep9_vm/src/arguments.rs:479 (list_element; abi/mod.rs create_list_from_array_like)",
        off: Off::Switch,
        what: "CreateListFromArrayLike reads a present Array or arguments element in place \
               (switched from M7-P0 fixer r2, Codex r1 CALLHOST-1)",
    },
    // ---- B0, a guard form ----
    FastPath {
        id: "t0-call",
        site: "crates/ep9_vm/src/interp/ops.rs:402 (call_fast; x86-64 form call.rs:960)",
        off: Off::Guard,
        what: "the T0 JS-to-JS call: the frame pushed in the handler",
    },
    FastPath {
        id: "t0-operators",
        site: "crates/ep9_vm/src/profile.rs:513-666 (the T0 operator fast bodies); interp/slow/ops_misc.rs:130, :137",
        off: Off::Guard,
        what: "int32 and number pairs of the arithmetic, compare, ToBoolean and strict \
               equality handlers",
    },
    FastPath {
        id: "ic-named",
        site: "crates/ep9_vm/src/ic.rs:1939 (IC hits)",
        off: Off::Guard,
        what: "a named load or store answered by the inline cache",
    },
    FastPath {
        id: "keyed-access",
        site: "crates/ep9_vm/src/profile.rs:870, :902 (the T0 keyed fast body)",
        off: Off::Guard,
        what: "an int32 key on a dense element store",
    },
    FastPath {
        id: "define-own",
        site: "crates/ep9_vm/src/property.rs:14, :783 (define)",
        off: Off::Guard,
        what: "CreateDataPropertyOrThrow: a new key on an extensible object, or a configurable \
               data property",
    },
    FastPath {
        id: "iter-array",
        site: "crates/ep9_vm/src/interp/slow/iter.rs:17, :67 (Builtin::Array); abi/iter.rs:16",
        off: Off::Guard,
        what: "GetIterator of an Array with %Array.prototype.values%: an element copy",
    },
    FastPath {
        id: "iter-string",
        site: "crates/ep9_vm/src/interp/slow/iter.rs:67 (Builtin::String)",
        off: Off::Guard,
        what: "GetIterator of a string primitive: a code point split",
    },
    FastPath {
        id: "array-tostring",
        site: "crates/ep9_vm/src/natives.rs:1404",
        off: Off::Guard,
        what: "Array.prototype.toString with the builtin join: join without a call",
    },
    FastPath {
        id: "instanceof",
        site: "crates/ep9_vm/src/instance_ic.rs:336; property.rs:1269",
        off: Off::Guard,
        what: "instanceof with the builtin @@hasInstance: OrdinaryHasInstance in place",
    },
    FastPath {
        id: "regexp-exec",
        site: "crates/ep9_builtins/src/regexp.rs:24; regexp/exec.rs:373",
        off: Off::Guard,
        what: "RegExpExec with the builtin exec: RegExpBuiltinExec into a MatchRecord",
    },
    FastPath {
        id: "regexp-split",
        site: "crates/ep9_builtins/src/regexp/symbols.rs:11, :794",
        off: Off::Guard,
        what: "@@split with %RegExp% and the builtin exec: one search per piece",
    },
    FastPath {
        id: "regexp-flags",
        site: "crates/ep9_builtins/src/regexp/symbols.rs:45, :60, :961; ep9_vm regexp.rs:480, :518",
        off: Off::Guard,
        what: "the flags text from [[RegExpRecord]] for a RegExp with its initial shape (RX0b)",
    },
    FastPath {
        id: "regexp-subject",
        site: "crates/ep9_builtins/src/regexp/symbols.rs:182, :276, :961; regexp/exec.rs:58",
        off: Off::Guard,
        what: "the subject S read in place by range (RX0b)",
    },
    FastPath {
        id: "json-string",
        site: "crates/ep9_builtins/src/json/parse.rs:743",
        off: Off::Guard,
        what: "a JSON string with no escape: a slice of the text",
    },
    // ---- M7-P1 OBJ1 (the literal clone) ----
    FastPath {
        id: "nested-literal",
        site: "crates/ep9_vm/src/literal_site.rs:732 (nested_object; NestedPlan, M7-P1 OBJ1)",
        off: Off::Guard,
        what: "an object literal with flat nested arrays built from the recorded shape and the checked boilerplate",
    },
];

/// Prints `typeof this`, the count and the joined arguments (the forwarded callee).
const SHOW: &str = "function show() { return [this === undefined ? 'u' : \
                    (this === globalThis ? 'g' : typeof this), arguments.length, \
                    Array.prototype.join.call(arguments, ',')].join('|'); }\n";

const CASES: &[Case] = &[
    // ---- host-entry ----
    Case {
        path: "host-entry",
        name: "the 9 shapes",
        src: "var o = { m: Math.max, s: 'abc' };\n\
              print(Math.max(), Math.max(1), Math.max(1, 2), Math.max(1, 2, 3));\n\
              print(o.m(), o.m(4), o.m(4, 5), o.m(4, 5, 6, 7));\n\
              var max = Math.max; print(max(), max(8), max(8, 9), max(8, 9, 10));\n\
              print('abcdef'.slice(1), 'abcdef'.slice(1, 3), parseInt('12'), String(5));",
        spec: None,
    },
    Case {
        path: "host-entry",
        name: "a host throw and its stack",
        src: "function f() { return null.x; }\n\
              try { Array.prototype.forEach.call(null, f); } catch (e) { print(e.name, e.message); }\n\
              try { [1].forEach(f); } catch (e) { print(e.name, e.stack.split('\\n').length); }\n\
              try { Symbol().toString.call(1); } catch (e) { print(e.name); }",
        spec: None,
    },
    Case {
        path: "host-entry",
        name: "receivers and realms",
        src: "var s = Object.prototype.toString;\n\
              print(s.call(1), s.call('x'), s.call(null), s.call(undefined), s.call([]));\n\
              var o = { t: s }; print(o.t(), [1, 2].join(), [1, [2, 3]].toString());",
        spec: None,
    },
    // ---- forward-apply (task 4 cases) ----
    Case {
        path: "forward-apply",
        name: "mapped and unmapped arguments",
        src: "function g(a, b) { return show.apply(this, arguments); }\n\
              function h(a, b) { a = 9; return show.apply(this, arguments); }\n\
              function k(a, b) { 'use strict'; a = 9; return show.apply(this, arguments); }\n\
              function m(a) { arguments[0] = 'M'; return show.apply(this, arguments); }\n\
              var o = { g: g, h: h, k: k, m: m };\n\
              for (var i = 0; i < 3; i++) print(o.g(1, 2, 3), o.h(1, 2), o.k(1, 2), o.m(1, 2), g());",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "length changed",
        src: "function a() { arguments.length = 5; return show.apply(this, arguments); }\n\
              function b() { arguments.length = 1; return show.apply(this, arguments); }\n\
              function c() { arguments.length = '2'; return show.apply(this, arguments); }\n\
              function d() { delete arguments.length; return show.apply(this, arguments); }\n\
              for (var i = 0; i < 3; i++) print(a(1, 2), b(1, 2), c(1, 2, 3), d(1, 2));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "an index deleted and proto elements",
        src: "function del() { delete arguments[1]; return show.apply(this, arguments); }\n\
              for (var i = 0; i < 2; i++) print(del(1, 2, 3));\n\
              Object.prototype[1] = 'P'; Array.prototype[0] = 'A';\n\
              print(del(1, 2, 3), show.apply(null, [, 2]), show.apply(null, [1, , 3]));\n\
              delete Object.prototype[1]; delete Array.prototype[0];\n\
              print(del(1, 2, 3), show.apply(null, [, 2]));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "a getter on an index",
        src: "var n = 0;\n\
              function g() { Object.defineProperty(arguments, 0, { get: function () { n++; return 'G'; } }); return show.apply(this, arguments); }\n\
              var arr = [1, 2]; Object.defineProperty(arr, 1, { get: function () { n++; return 'H'; } });\n\
              for (var i = 0; i < 2; i++) print(g(1, 2), show.apply(null, arr), n);",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "holes and array-likes",
        src: "var holey = [1, , 3]; var sparse = []; sparse[3] = 'x';\n\
              for (var i = 0; i < 2; i++) {\n\
                print(show.apply(null, holey), show.apply(null, sparse), show.apply(null, { length: 2, 0: 'a', 1: 'b' }));\n\
                print(show.apply(null, new String('xy')), show.apply(null, { length: 1.5, 0: 'z' }), show.apply(null, { length: -1 }));\n\
              }\n\
              try { show.apply(null, 1); } catch (e) { print(e.name); }\n\
              try { show.apply(null, 'ab'); } catch (e) { print(e.name); }",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "a Proxy",
        src: "var log = [];\n\
              var p = new Proxy([1, 2], { get: function (t, k) { log.push(String(k)); return t[k]; } });\n\
              for (var i = 0; i < 2; i++) print(show.apply(null, p), log.join());\n\
              var pf = new Proxy(show, {}); print(pf.apply(null, [1]), Function.prototype.apply.call(pf, null, [2]));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "bound functions",
        src: "var b = show.bind({ x: 1 }, 'B');\n\
              var ap = Function.prototype.apply.bind(show);\n\
              for (var i = 0; i < 2; i++) print(b.apply(null, [1]), b.apply(), ap(null, [7, 8]), ap({}, null));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "0, 1 and many arguments",
        src: "var big = []; for (var i = 0; i < 1000; i++) big.push(i);\n\
              function sum() { var s = 0; for (var i = 0; i < arguments.length; i++) s += arguments[i]; return s; }\n\
              function via() { return sum.apply(null, arguments); }\n\
              for (var i = 0; i < 2; i++) print(show.apply(null), show.apply(null, null), show.apply(null, []),\n\
                show.apply(null, [1]), sum.apply(null, big), via(1, 2, 3, 4, 5, 6, 7, 8, 9, 10));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "this binding and callee kinds",
        src: "function sloppy() { return typeof this; }\n\
              function strict() { 'use strict'; return this === undefined ? 'u' : typeof this; }\n\
              function* gen(a, b) { yield a; yield b; }\n\
              var arrow = () => typeof this; class C {}\n\
              for (var i = 0; i < 2; i++) {\n\
                print(sloppy.apply(5), sloppy.apply('s', []), sloppy.apply(null), strict.apply(5), strict.apply(undefined, [1]));\n\
                var it = gen.apply(null, [1, 2]); print(it.next().value, it.next().value, arrow.apply(1));\n\
                try { C.apply(null, []); } catch (e) { print(e.name, e.message); }\n\
                try { Function.prototype.apply.call(1, null, []); } catch (e) { print(e.name, e.message); }\n\
                print(Math.max.apply(null, [1, 5, 3]), Reflect.apply(show, 1, [2]));\n\
              }",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "a stack overflow gives RangeError",
        src: "function r(n) { return n === 0 ? 0 : 1 + r.apply(null, [n - 1]); }\n\
              function q() { return q.apply(this, arguments); }\n\
              try { r(1e7); } catch (e) { print(e.name, e.message); }\n\
              try { q(1, 2); } catch (e) { print(e.name, e.message); }\n\
              print(r(500));",
        spec: None,
    },
    // ---- forward-call ----
    Case {
        path: "forward-call",
        name: "this, counts and callee kinds",
        src: "function sloppy() { return typeof this; }\n\
              function strict() { 'use strict'; return this === undefined ? 'u' : typeof this; }\n\
              class C {}\n\
              for (var i = 0; i < 3; i++) {\n\
                print(show.call(), show.call(null), show.call({}, 1), show.call(1, 2, 3), show.call(null, 1, 2, 3, 4, 5));\n\
                print(sloppy.call(5), sloppy.call(undefined), strict.call(5), strict.call());\n\
                try { C.call(null); } catch (e) { print(e.name, e.message); }\n\
                try { Function.prototype.call.call({}); } catch (e) { print(e.name, e.message); }\n\
                print(Math.max.call(null, 4, 2), show.bind(1).call(2, 3), Function.prototype.call.call(show, 7, 8));\n\
              }",
        spec: None,
    },
    Case {
        path: "forward-call",
        name: "the deltablue form and a stack overflow",
        src: "function Base(s) { this.s = s; } Base.prototype.get = function () { return this.s; };\n\
              function D(s, t) { D.sup.call(this, s); this.t = t; } D.sup = Base;\n\
              D.prototype = Object.create(Base.prototype); D.prototype.get = function () { return Base.prototype.get.call(this) + this.t; };\n\
              for (var i = 0; i < 3; i++) print(new D(i, 'x').get());\n\
              function r(n) { return n === 0 ? 0 : 1 + r.call(null, n - 1); }\n\
              try { r(1e7); } catch (e) { print(e.name, e.message); }\n\
              print(r(500));",
        spec: None,
    },
    // ---- args-create ----
    Case {
        path: "args-create",
        name: "the object made each way",
        src: "function m(a, b) { return arguments; }\n\
              function u(a, b) { 'use strict'; return arguments; }\n\
              for (var i = 0; i < 3; i++) {\n\
                var x = m(1, 2, 3), y = u(1, 2);\n\
                print(Object.getOwnPropertyNames(x).join(), Object.getOwnPropertyNames(y).join(), x.length, y.length);\n\
                print(JSON.stringify(Object.getOwnPropertyDescriptor(x, 'callee').writable), typeof x[Symbol.iterator], [...x].join());\n\
                try { y.callee; } catch (e) { print(e.name); }\n\
                print(Object.prototype.toString.call(x), Object.getPrototypeOf(y) === Object.prototype);\n\
              }",
        spec: None,
    },
    // ---- clfal-length ----
    Case {
        path: "clfal-length",
        name: "Reflect.apply and apply with an arguments object",
        src: "function f() { return Reflect.apply(show, null, arguments); }\n\
              function g() { arguments.length = 2; return Reflect.apply(show, null, arguments); }\n\
              function h() { Object.defineProperty(arguments, 'length', { get: function () { return 1; } }); return Reflect.apply(show, null, arguments); }\n\
              for (var i = 0; i < 3; i++) print(f(1, 2, 3), g(1, 2, 3), h(1, 2, 3), f());",
        spec: None,
    },
    Case {
        path: "clfal-elements",
        name: "in place or through [[Get]]",
        src: "function f() { return Reflect.apply(show, null, arguments); }\n\
              function g() { delete arguments[1]; return Reflect.apply(show, null, arguments); }\n\
              var a = [4, 5]; Object.defineProperty(a, 0, { get: function () { return 6; } });\n\
              Object.prototype[1] = 'p'; var h = [7]; h.length = 2;\n\
              print(f(1, 2, 3), g(1, 2, 3), Reflect.apply(show, null, [4, 5]), Reflect.apply(show, null, a), Reflect.apply(show, null, h));\n\
              delete Object.prototype[1];",
        spec: None,
    },
    // ---- the Guard rows ----
    Case {
        path: "t0-call",
        name: "a JS call in the handler or from Rust",
        src: "function f(a, b) { return [typeof this, a, b, arguments.length].join(); }\n\
              var o = { f: f }; print(o.f(1), o.f(1, 2, 3), f(), f.length);",
        spec: Some(
            "function f(a, b) { return [typeof this, a, b, arguments.length].join(); }\n\
             var o = { f: f }; print(Reflect.apply(f, o, [1]), Reflect.apply(f, o, [1, 2, 3]), Reflect.apply(f, undefined, []), f.length);",
        ),
    },
    Case {
        path: "t0-operators",
        name: "number pairs or ToPrimitive",
        src: "var a = 7, b = 3;\n\
              print(a + b, a - b, a * b, a / b, a % b, a ** b, a | b, a & b, a ^ b, a << b, a >> b, a >>> b);\n\
              print(a < b, a <= b, a > b, a >= b, a === b, a == b, !a, -a, ~a);\n\
              var c = 2147483647; print(c + 1, -c - 2, c * 2, 0 * -1, 1 / (0 * -1));",
        spec: Some(
            "var a = { valueOf: function () { return 7; } }, b = new Number(3);\n\
             print(a + b, a - b, a * b, a / b, a % b, a ** b, a | b, a & b, a ^ b, a << b, a >> b, a >>> b);\n\
             print(a < b, a <= b, a > b, a >= b, a.valueOf() === b.valueOf(), a == b, !a.valueOf(), -a, ~a);\n\
             var c = new Number(2147483647); print(c + 1, -c - 2, c * 2, 0 * -1, 1 / (0 * -1));",
        ),
    },
    Case {
        path: "ic-named",
        name: "an IC hit or Reflect.get and Reflect.set",
        src: "function P(x) { this.x = x; } var s = 0;\n\
              for (var i = 0; i < 20; i++) { var p = new P(i); p.y = p.x + 1; s += p.x + p.y; }\n\
              print(s);",
        spec: Some(
            "function P(x) { Reflect.set(this, 'x', x); } var s = 0;\n\
             for (var i = 0; i < 20; i++) { var p = new P(i); Reflect.set(p, 'y', Reflect.get(p, 'x') + 1); s += Reflect.get(p, 'x') + Reflect.get(p, 'y'); }\n\
             print(s);",
        ),
    },
    Case {
        path: "ic-named",
        name: "a megamorphic load (the stub cache probe of the GetNamedProperty handler) or Reflect.get",
        src: "function mk(i) { var o = {}; o['p' + i] = i; o.x = i; return o; }\n\
              function getx(o) { return o.x; }\n\
              var os = []; for (var i = 0; i < 12; i++) os.push(mk(i));\n\
              var s = 0; for (var r = 0; r < 30; r++) for (var i = 0; i < 12; i++) s += getx(os[i]);\n\
              var P = { x: 100 }; var c = Object.create(P); c.k = 1; s += getx(c) + getx(c);\n\
              P.x = 7; s += getx(c); Object.defineProperty(c, 'x', { value: 9 }); s += getx(c);\n\
              print(s, getx({}), getx(Object.create(null)), getx('str'), getx(5));",
        spec: Some(
            "function mk(i) { var o = {}; o['p' + i] = i; o.x = i; return o; }\n\
             function getx(o) { return Reflect.get(Object(o), 'x', o); }\n\
             var os = []; for (var i = 0; i < 12; i++) os.push(mk(i));\n\
             var s = 0; for (var r = 0; r < 30; r++) for (var i = 0; i < 12; i++) s += getx(os[i]);\n\
             var P = { x: 100 }; var c = Object.create(P); c.k = 1; s += getx(c) + getx(c);\n\
             P.x = 7; s += getx(c); Object.defineProperty(c, 'x', { value: 9 }); s += getx(c);\n\
             print(s, getx({}), getx(Object.create(null)), getx('str'), getx(5));",
        ),
    },
    Case {
        path: "keyed-access",
        name: "an int32 key or a string key",
        src: "var a = [1, 2, 3]; var s = 0; for (var i = 0; i < 3; i++) { a[i] = a[i] * 2; s += a[i]; }\n\
              print(s, a.join(), a[5], a[-1]);",
        spec: Some(
            "var a = [1, 2, 3]; var s = 0; for (var i = 0; i < 3; i++) { a[String(i)] = a[String(i)] * 2; s += a[String(i)]; }\n\
             print(s, a.join(), a['5'], a['-1']);",
        ),
    },
    Case {
        path: "define-own",
        name: "a literal or defineProperty",
        src: "var o = { a: 1, b: 2 }; o.c = 3;\n\
              print(JSON.stringify(Object.getOwnPropertyDescriptors(o)), Object.keys(o).join());",
        spec: Some(
            "var o = {}; var d = { writable: true, enumerable: true, configurable: true };\n\
             Object.defineProperty(o, 'a', Object.assign({ value: 1 }, d)); Object.defineProperty(o, 'b', Object.assign({ value: 2 }, d));\n\
             Object.defineProperty(o, 'c', Object.assign({ value: 3 }, d));\n\
             print(JSON.stringify(Object.getOwnPropertyDescriptors(o)), Object.keys(o).join());",
        ),
    },
    Case {
        path: "iter-array",
        name: "spread and for-of of an Array",
        src: "var a = [1, , 3]; var out = []; for (var v of a) out.push(v);\n\
              print([...a].length, out.join(), Array.from(a).join(), [...[]].length);",
        spec: Some(
            "var a = [1, , 3]; a[Symbol.iterator] = function () { return Array.prototype.values.call(this); };\n\
             var out = []; for (var v of a) out.push(v);\n\
             print([...a].length, out.join(), Array.from(a).join(), [...[]].length);",
        ),
    },
    Case {
        path: "iter-string",
        name: "spread of a string",
        src: "var s = 'a\\u{1F600}b\\uD800c'; print([...s].length, [...s].map(function (c) { return c.length; }).join());",
        spec: Some(
            "var s = new String('a\\u{1F600}b\\uD800c'); print([...s].length, [...s].map(function (c) { return c.length; }).join());",
        ),
    },
    Case {
        path: "array-tostring",
        name: "the builtin join or a wrapper",
        src: "print([1, [2, 3], null, undefined].toString(), String([4, 5]));",
        spec: Some(
            "var a = [1, [2, 3], null, undefined]; a.join = function () { return Array.prototype.join.call(this); };\n\
             var b = [4, 5]; b.join = function () { return Array.prototype.join.call(this); };\n\
             print(a.toString(), String(b));",
        ),
    },
    Case {
        path: "instanceof",
        name: "the builtin @@hasInstance or a wrapper",
        src: "function F() {} var o = new F();\n\
              print(o instanceof F, o instanceof Object, 1 instanceof F, F instanceof Function);",
        spec: Some(
            "var h = Function.prototype[Symbol.hasInstance];\n\
             function F() {} var o = new F();\n\
             Object.defineProperty(F, Symbol.hasInstance, { value: function (v) { return h.call(this, v); } });\n\
             print(o instanceof F, h.call(Object, o), 1 instanceof F, h.call(Function, F));",
        ),
    },
    Case {
        path: "regexp-exec",
        name: "the builtin exec or a wrapper",
        src: "var r = /(a)(b)?/g; var m = r.exec('xab'); print(m.index, m.join(), r.lastIndex);\n\
              print('aXa'.replace(/a/g, 'b'), /a/.test('ba'), 'aba'.match(/a/g).join(), 'xa'.search(/a/));",
        spec: Some(
            "var e = RegExp.prototype.exec; function w(rx) { rx.exec = function (s) { return e.call(this, s); }; return rx; }\n\
             var r = w(/(a)(b)?/g); var m = r.exec('xab'); print(m.index, m.join(), r.lastIndex);\n\
             print('aXa'.replace(w(/a/g), 'b'), w(/a/).test('ba'), 'aba'.match(w(/a/g)).join(), 'xa'.search(w(/a/)));",
        ),
    },
    Case {
        path: "regexp-split",
        name: "one search per piece or a sticky exec per position",
        src: "print('a,b,,c'.split(/,/).join('|'), 'abc'.split(/(b)/).join('|'), 'a1b2'.split(/\\d/, 1).join('|'), ''.split(/x/).length);",
        spec: Some(
            "var e = RegExp.prototype.exec; RegExp.prototype.exec = function (s) { return e.call(this, s); };\n\
             print('a,b,,c'.split(/,/).join('|'), 'abc'.split(/(b)/).join('|'), 'a1b2'.split(/\\d/, 1).join('|'), ''.split(/x/).length);",
        ),
    },
    Case {
        path: "regexp-flags",
        name: "the flags text from the record or the getters",
        src: "var r = /a/gimsuy; print(r.flags, 'aAa'.replace(/a/gi, 'x'), 'aa'.match(/a/g).length);",
        spec: Some(
            "var r = /a/gimsuy; r.own = 1; var s = /a/gi; s.own = 1; var t = /a/g; t.own = 1;\n\
             print(r.flags, 'aAa'.replace(s, 'x'), 'aa'.match(t).length);",
        ),
    },
    Case {
        path: "regexp-subject",
        name: "S read by range or copied whole (GetSubstitution)",
        src: "print('abcabc'.replace(/b/g, function (m) { return '[' + m + ']'; }),\n\
               'abcabc'.replace(/b/g, function (m, i, s) { return '[' + s.slice(0, i) + '|' + s.slice(i + 1) + ']'; }));",
        spec: Some("print('abcabc'.replace(/b/g, '[$&]'), 'abcabc'.replace(/b/g, \"[$`|$']\"));"),
    },
    Case {
        path: "json-string",
        name: "no escape or an escape",
        src: "var v = JSON.parse('{\"ab\":\"cd\",\"e\":[\"f\",\"\"]}'); print(v.ab, v.e.join('|'), JSON.parse('\"xyz\"'));",
        spec: Some(
            "var v = JSON.parse('{\"a\\\\u0062\":\"c\\\\u0064\",\"e\":[\"\\\\u0066\",\"\"]}'); print(v.ab, v.e.join('|'), JSON.parse('\"x\\\\u0079z\"'));",
        ),
    },
    Case {
        path: "nested-literal",
        name: "flat nested arrays or computed ones",
        src: "function f(t) { return { a: [1, 2.5, , 's'], b: t, c: [7, 8] }; }\n\
              var p = f(0), o; for (var i = 1; i < 20; i++) { o = f(i); o.c[0] += i; }\n\
              print(JSON.stringify(o), o.a === p.a, 2 in o.a, o.c[0] + p.c[0], Object.keys(o).join());",
        spec: Some(
            "function f(t) { var one = 1; return { a: [one, 2.5, , 's'], b: t, c: [one * 7, 8] }; }\n\
             var p = f(0), o; for (var i = 1; i < 20; i++) { o = f(i); o.c[0] += i; }\n\
             print(JSON.stringify(o), o.a === p.a, 2 in o.a, o.c[0] + p.c[0], Object.keys(o).join());",
        ),
    },
];

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// What a run gives: the printed lines and the uncaught exception.
type RunOut = (Vec<String>, Option<String>);

/// Runs the sloppy script `SHOW + src` on a fresh isolate with the human fast paths `on` or
/// off; `stress`: a collection at each allocation, then the heap verifier. On a big stack
/// (debug builds and the overflow cases).
fn run(src: &'static str, on: bool, stress: bool) -> RunOut {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let text = format!("{SHOW}{src}");
            let unit = ep9_frontend::compile(
                &text,
                SourceOpts::script(Strictness::Sloppy),
                CompileOptions::default(),
            )
            .expect("the case compiles");
            let mut iso = Isolate::new(IsolateConfig {
                heap: HeapConfig {
                    gc_stress: u32::from(stress),
                    verify_after_gc: stress,
                    ..HeapConfig::default()
                },
                builtins: Some(&ep9_builtins::TABLE),
                ..IsolateConfig::default()
            })
            .expect("an isolate");
            iso.register_host_function("print", 1, print);
            OUT.with(|o| o.borrow_mut().clear());
            let r = iso.with_cx_checkpoint(|cx| {
                cx.set_human_fast_paths(on);
                cx.run_script(unit, "case.js").map(|_| ())
            });
            let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            // The switch is process-wide and keyed by the isolate id: turn it on again, so
            // a dropped isolate leaves no id behind (else every later host call takes the
            // lock; M7-P0 JIT reviewer F5).
            let _ = iso.with_cx(|cx| {
                cx.set_human_fast_paths(true);
                Ok(())
            });
            (out, r.err().map(|e| e.message))
        })
        .expect("a thread")
        .join()
        .expect("no panic")
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The ep9 commit of this tree (a JSON string) and whether the tree has uncommitted changes to
/// tracked files (a JSON bool), from git; each is `null` when git cannot say (no git, no repo).
fn git_state() -> (String, String) {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(env!("CARGO_MANIFEST_DIR"))
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "HEAD"]).map_or("null".to_string(), |c| json_str(&c));
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .map_or("null".to_string(), |s| (!s.is_empty()).to_string());
    (commit, dirty)
}

fn report_path() -> PathBuf {
    match std::env::var_os("EP9_FASTDIFF_JSON") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fastdiff.json"),
    }
}

#[test]
fn every_fast_path_has_a_case_and_every_case_a_fast_path() {
    for p in INVENTORY {
        let n = CASES.iter().filter(|c| c.path == p.id).count();
        assert!(n >= 1, "fast path {} has no case", p.id);
    }
    for c in CASES {
        let p = INVENTORY.iter().find(|p| p.id == c.path);
        let p = p.unwrap_or_else(|| panic!("case {:?} names no fast path {}", c.name, c.path));
        assert_eq!(
            c.spec.is_some(),
            p.off == Off::Guard,
            "case {:?}: a Guard row needs its spec form, a Switch row none",
            c.name
        );
    }
    let mut ids: Vec<&str> = INVENTORY.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), INVENTORY.len(), "an id twice");
}

/// Runs case `c` on and off (and its spec form, and for a Switch row under GC stress with the
/// fast paths on): the mismatches and the number of runs.
fn check_case(c: &Case) -> (Vec<String>, usize) {
    let started = std::time::Instant::now();
    let p = INVENTORY.iter().find(|p| p.id == c.path).expect("a row");
    let label = format!("{} / {}", c.path, c.name);
    let mut mismatches = Vec::new();
    let on = run(c.src, true, false);
    let off = run(c.src, false, false);
    let mut runs = 2;
    if on.1.is_some() {
        mismatches.push(format!("{label}: uncaught {:?}", on.1));
    }
    if on != off {
        mismatches.push(format!("{label}: on {on:?}\n  off {off:?}"));
    }
    if let Some(spec) = c.spec {
        let s = run(spec, true, false);
        let s_off = run(spec, false, false);
        runs += 2;
        if s != on {
            mismatches.push(format!("{label}: fast {on:?}\n  spec {s:?}"));
        }
        if s != s_off {
            mismatches.push(format!("{label}: spec on {s:?}\n  spec off {s_off:?}"));
        }
    }
    // A Switch row's fast path under GC stress (a collection at each allocation). The stack
    // overflow cases fill the VM stack, and each collection would scan it: no stress there.
    if p.off == Off::Switch && !c.name.contains("stack overflow") {
        let st = run(c.src, true, true);
        runs += 1;
        if st != on {
            mismatches.push(format!("{label}: GC stress: {st:?}\n  want {on:?}"));
        }
    }
    eprintln!("{label}: {:.1} s", started.elapsed().as_secs_f64());
    (mismatches, runs)
}

#[test]
fn every_fast_path_matches_its_spec_steps() {
    if let Some(dir) = std::env::var_os("EP9_FASTDIFF_DUMP") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, c) in CASES.iter().enumerate() {
            let print = "var print = typeof print === 'function' ? print : console.log;\n";
            std::fs::write(
                dir.join(format!("{i:02}-fast.js")),
                format!("{print}{SHOW}{}", c.src),
            )
            .unwrap();
            if let Some(spec) = c.spec {
                std::fs::write(
                    dir.join(format!("{i:02}-spec.js")),
                    format!("{print}{SHOW}{spec}"),
                )
                .unwrap();
            }
        }
    }
    let only = std::env::var("EP9_FASTDIFF_ONLY").ok();
    let todo: Vec<&Case> = CASES
        .iter()
        .filter(|c| {
            only.as_deref()
                .is_none_or(|o| c.path.contains(o) || c.name.contains(o))
        })
        .collect();
    assert!(
        !todo.is_empty(),
        "EP9_FASTDIFF_ONLY={only:?} matches no case"
    );
    // The cases run in parallel (each run on its own isolate and thread).
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(c) = todo.get(i) else { break };
                    let r = check_case(c);
                    results.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    let mut mismatches = Vec::new();
    let mut per_path = std::collections::BTreeMap::<&str, usize>::new();
    let mut runs = 0;
    for (i, (m, n)) in results {
        *per_path.entry(todo[i].path).or_default() += m.len();
        mismatches.extend(m);
        runs += n;
    }
    let (commit, dirty) = git_state();
    let mut j = format!(
        "{{\"schema\":\"ep9-fastdiff/v1\",\"commit\":{commit},\"dirty\":{dirty},\"fast_paths\":["
    );
    for (i, p) in INVENTORY.iter().enumerate() {
        // With EP9_FASTDIFF_ONLY, a path whose cases did not run has 0 cases in this report.
        let n = todo.iter().filter(|c| c.path == p.id).count();
        let off = if p.off == Off::Switch {
            "switch"
        } else {
            "guard"
        };
        let _ = write!(
            j,
            "{}{{\"name\":{},\"site\":{},\"off\":\"{off}\",\"cases\":{n},\"mismatches\":{},\"what\":{}}}",
            if i > 0 { "," } else { "" },
            json_str(p.id),
            json_str(p.site),
            per_path.get(p.id).copied().unwrap_or(0),
            json_str(p.what)
        );
    }
    let _ = write!(
        j,
        "],\"paths\":{},\"cases\":{},\"runs\":{runs},\"mismatches\":{},\"details\":[",
        INVENTORY.len(),
        todo.len(),
        mismatches.len()
    );
    for (i, m) in mismatches.iter().enumerate() {
        let _ = write!(j, "{}{}", if i > 0 { "," } else { "" }, json_str(m));
    }
    j.push_str("]}\n");
    let path = report_path();
    std::fs::write(&path, &j).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("FASTDIFF report: {}\n{j}", path.display());
    assert!(
        mismatches.is_empty(),
        "{} FASTDIFF mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
