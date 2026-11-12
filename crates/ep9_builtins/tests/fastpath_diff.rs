//! FASTDIFF: the fast-path differential tests (09-milestones.md:108 "every human fast path
//! matches its spec-layer path: 0 mismatches"; 10-quality.md:71, every PR from M2; plan v12
//! V12-D30 and rule 73; M7-P0 CALLHOST).
//!
//! A human fast path is hand-written code that skips spec steps under a guard, with the spec
//! steps beside it in the same build. [`INVENTORY`] lists every one at B0 (the grep
//! `fast path` over `ep9_vm` and `ep9_builtins`, the RX0b list of `regexp/symbols.rs` and the
//! JSON string slice of `json/parse.rs`), the fast paths that the same "is this still the
//! intrinsic?" test guards (`builtins::existing_function`, `Cx::is_builtin`), the
//! M7-P0 CALLHOST paths and the M7-P3 CONSTRUCT row. Each row says how its spec steps run:
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
//! Route P0-FASTHITS: a run also gives the hit count of each Switch fast path (ep9_vm feature
//! `fastdiff-hits`, `internals::take_fastdiff_hits`). Over its cases, a Switch row must count at
//! least 1 hit with the fast paths on and 0 with them off: 0 mismatches from cases that never
//! took the fast path is no evidence.
//!
//! The JSON report (schema `ep9-fastdiff/v1`, the form that `cargo xtask m2-exit` item 5
//! reads): the ep9 commit and whether its tree was dirty (`commit`, `dirty`; `null` when git
//! cannot say), `fast_paths[]` with each fast path's `name`, `cases` and `mismatches` (and for
//! a Switch row `hits_on` and `hits_off`), and the totals. It goes to `$EP9_FASTDIFF_JSON` if set, else to
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
        site: "crates/ep9_vm/src/interp/slow/call.rs forward (apply); crates/ep9_vm/src/call.rs \
               forward_frame (the T0 form in interp/ops.rs call_fast, M7-P4 LEVER-3; x86-64 \
               without spike-native: interp/ops.rs forward_or_call, M8-P0)",
        off: Off::Switch,
        what: "f.apply(thisArg, list) of a bytecode f with an unmodified arguments object, a \
               packed Array, undefined or null: the frame pushed from the source",
    },
    FastPath {
        id: "forward-call",
        site: "crates/ep9_vm/src/interp/slow/call.rs forward (call); crates/ep9_vm/src/call.rs \
               forward_frame (the T0 form in interp/ops.rs call_fast, M7-P4 LEVER-3; x86-64 \
               without spike-native: interp/ops.rs forward_or_call, M8-P0)",
        off: Off::Switch,
        what: "f.call(thisArg, ...args) of a bytecode f: the frame pushed from the registers",
    },
    // ---- B0, the switch ----
    FastPath {
        id: "args-create",
        site: "crates/ep9_vm/src/arguments.rs:64 (create_fast, :247)",
        off: Off::Switch,
        what: "a new arguments object takes the realm's cached shape (V10-D11)",
    },
    FastPath {
        id: "clfal-length",
        site: "crates/ep9_vm/src/arguments.rs:506 (list_length)",
        off: Off::Switch,
        what: "CreateListFromArrayLike reads the length of an unmodified arguments object in \
               place",
    },
    FastPath {
        id: "clfal-elements",
        site: "crates/ep9_vm/src/arguments.rs:526 (list_element; abi/mod.rs create_list_from_array_like)",
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
    // ---- M7-P3 CONSTRUCT, a guard form ----
    FastPath {
        id: "construct-site",
        site: "crates/ep9_vm/src/construct_ic.rs fast (the Construct handler, interp/ops.rs)",
        off: Off::Guard,
        what: "`new F(...)` of a plain bytecode constructor at a monomorphic site: `this` from \
               the site's initial shape, the stub and callee frames written in the handler",
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
        site: "crates/ep9_vm/src/ic.rs:1964 (IC hits)",
        off: Off::Guard,
        what: "a named load or store answered by the inline cache",
    },
    FastPath {
        id: "ic-global",
        site: "crates/ep9_vm/src/ic.rs:693-770 (the global IC hits, M7-P2 GLOBALS)",
        off: Off::Guard,
        what: "a global load or store (LdaGlobal, LdaGlobalInsideTypeof, StaGlobal) answered by \
               the inline cache",
    },
    FastPath {
        id: "keyed-access",
        site: "crates/ep9_vm/src/interp/ops.rs:1166 (the GetKeyedProperty array path); keyed_ic.rs:161 (fast_set), :230 (the store grow case)",
        off: Off::Guard,
        what: "an int32 key on a dense element store; the StoreKeyed grow case (a hole fill, or an append at or past length inside the capacity, no kind transition)",
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
        site: "crates/ep9_vm/src/natives.rs:1546",
        off: Off::Guard,
        what: "Array.prototype.toString with the builtin join: join without a call",
    },
    FastPath {
        id: "instanceof",
        site: "crates/ep9_vm/src/instance_ic.rs:336; property.rs:1277",
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
    // ---- TypedArray TA-2 TA-KEYED, a guard form ----
    FastPath {
        id: "keyed-view",
        site: "crates/ep9_vm/src/keyed_ic.rs view::get, view::set (from fast_get and fast_set; \
               TypedArray TA-2 TA-KEYED)",
        off: Off::Guard,
        what: "an int32 key on a TypedArray: the element or undefined at GetKeyedProperty; at \
               SetKeyedProperty the in-bounds store of an int32 (integer types) or a Number \
               (Float32, Float64)",
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
    // ---- construct-site: the spec form first constructs other functions at each site, 20
    // times (after the threshold calls the caller has its vector), so the site is megamorphic
    // and every construct takes the cold path ----
    Case {
        path: "construct-site",
        name: "a plain constructor",
        src: "function P(x, y) { this.x = x; this.y = y; }\n\
              P.prototype.sum = function () { return this.x + this.y; };\n\
              function mk(C, a, b) { return new C(a, b); }\n\
              var s = 0, o; for (var i = 0; i < 30; i++) { o = mk(P, i, 2); s += o.sum(); }\n\
              print(s, Object.keys(o).join(), Object.getPrototypeOf(o) === P.prototype, o instanceof P, o.constructor === P, JSON.stringify(o));",
        spec: Some(
            "function P(x, y) { this.x = x; this.y = y; }\n\
             P.prototype.sum = function () { return this.x + this.y; };\n\
             function mk(C, a, b) { return new C(a, b); } for (var k = 0; k < 20; k++) mk(function D() {}, 0, 0);\n\
             var s = 0, o; for (var i = 0; i < 30; i++) { o = mk(P, i, 2); s += o.sum(); }\n\
             print(s, Object.keys(o).join(), Object.getPrototypeOf(o) === P.prototype, o instanceof P, o.constructor === P, JSON.stringify(o));",
        ),
    },
    Case {
        path: "construct-site",
        name: "missing and extra arguments, arguments and new.target",
        src: "function A(a, b, c) { this.n = arguments.length; this.t = new.target === A; this.v = [a, b, c].join('/'); this.s = show.apply(this, arguments); }\n\
              function mk0(C) { return new C(); } function mk1(C) { return new C(1); } function mk4(C) { return new C(1, 2, 3, 4); }\n\
              var r; for (var i = 0; i < 20; i++) { r = [mk0(A), mk1(A), mk4(A)]; }\n\
              print(JSON.stringify(r));",
        spec: Some(
            "function A(a, b, c) { this.n = arguments.length; this.t = new.target === A; this.v = [a, b, c].join('/'); this.s = show.apply(this, arguments); }\n\
             function mk0(C) { return new C(); } function mk1(C) { return new C(1); } function mk4(C) { return new C(1, 2, 3, 4); }\n\
             for (var k = 0; k < 20; k++) { mk0(function D() {}); mk1(function D() {}); mk4(function D() {}); }\n\
             var r; for (var i = 0; i < 20; i++) { r = [mk0(A), mk1(A), mk4(A)]; }\n\
             print(JSON.stringify(r));",
        ),
    },
    Case {
        path: "construct-site",
        name: "a new prototype, a prototype that is not an object, a returned object",
        src: "function P() { this.k = 1; }\n\
              function R(o) { this.k = 2; return o; }\n\
              function mk(C, a) { return new C(a); } function mkr(C, a) { return new C(a); }\n\
              var out = '', rs = [];\n\
              for (var i = 0; i < 30; i++) {\n\
                if (i === 10) P.prototype = { tag: 'b' };\n\
                if (i === 20) P.prototype = 5;\n\
                var p = mk(P); out += Object.getPrototypeOf(p) === Object.prototype ? 'O' : (p.tag || 'a');\n\
                var x = mkr(R, i % 3 === 0 ? { z: i } : (i % 3 === 1 ? i : null)); rs.push(x.z === undefined ? x.k : x.z);\n\
              }\n\
              print(out, rs.join());",
        spec: Some(
            "function P() { this.k = 1; }\n\
             function R(o) { this.k = 2; return o; }\n\
             function mk(C, a) { return new C(a); } function mkr(C, a) { return new C(a); }\n\
             for (var k = 0; k < 20; k++) { mk(function D() {}); mkr(function D() {}); }\n\
             var out = '', rs = [];\n\
             for (var i = 0; i < 30; i++) {\n\
               if (i === 10) P.prototype = { tag: 'b' };\n\
               if (i === 20) P.prototype = 5;\n\
               var p = mk(P); out += Object.getPrototypeOf(p) === Object.prototype ? 'O' : (p.tag || 'a');\n\
               var x = mkr(R, i % 3 === 0 ? { z: i } : (i % 3 === 1 ? i : null)); rs.push(x.z === undefined ? x.k : x.z);\n\
             }\n\
             print(out, rs.join());",
        ),
    },
    Case {
        path: "construct-site",
        name: "class constructors, field initializers and a derived class",
        src: "class B { constructor(x) { this.x = x; } get dbl() { return this.x * 2; } }\n\
              class F { a = 1; b = this.a + 1; constructor(c) { this.c = c; } }\n\
              class E extends B { constructor(x) { super(x); this.e = new.target === E; } }\n\
              function mkB(C, a) { return new C(a); } function mkF(C, a) { return new C(a); } function mkE(C, a) { return new C(a); }\n\
              var s = 0, f, e; for (var i = 0; i < 20; i++) { s += mkB(B, i).dbl; f = mkF(F, i); e = mkE(E, i); }\n\
              print(s, JSON.stringify(f), JSON.stringify(e), e instanceof B, Object.keys(f).join());",
        spec: Some(
            "class B { constructor(x) { this.x = x; } get dbl() { return this.x * 2; } }\n\
             class F { a = 1; b = this.a + 1; constructor(c) { this.c = c; } }\n\
             class E extends B { constructor(x) { super(x); this.e = new.target === E; } }\n\
             function mkB(C, a) { return new C(a); } function mkF(C, a) { return new C(a); } function mkE(C, a) { return new C(a); }\n\
             for (var k = 0; k < 20; k++) { mkB(class D {}); mkF(class D {}); mkE(class D {}); }\n\
             var s = 0, f, e; for (var i = 0; i < 20; i++) { s += mkB(B, i).dbl; f = mkF(F, i); e = mkE(E, i); }\n\
             print(s, JSON.stringify(f), JSON.stringify(e), e instanceof B, Object.keys(f).join());",
        ),
    },
    Case {
        path: "construct-site",
        name: "a throw in the constructor and a stack overflow of constructs",
        src: "function T(n) { if (n === 15) throw new RangeError('at ' + n); this.n = n; }\n\
              function mk(C, a) { return new C(a); }\n\
              var r = []; for (var i = 0; i < 20; i++) { try { r.push(mk(T, i).n); } catch (e) { r.push(e.name + ':' + e.message); } }\n\
              function D(n) { this.d = n > 0 ? mkd(D, n - 1) : null; }\n\
              function mkd(C, a) { return new C(a); }\n\
              var depth = 'none'; try { mkd(D, 1e6); } catch (e) { depth = e.name; }\n\
              print(r.join(), depth, mkd(D, 3).d.d.d.d);",
        spec: Some(
            "function T(n) { if (n === 15) throw new RangeError('at ' + n); this.n = n; }\n\
             function mk(C, a) { return new C(a); } for (var k = 0; k < 20; k++) mk(function X() {});\n\
             var r = []; for (var i = 0; i < 20; i++) { try { r.push(mk(T, i).n); } catch (e) { r.push(e.name + ':' + e.message); } }\n\
             function D(n) { this.d = n > 0 ? mkd(D, n - 1) : null; }\n\
             function mkd(C, a) { return new C(a); } for (var k = 0; k < 20; k++) mkd(function X() {});\n\
             var depth = 'none'; try { mkd(D, 1e6); } catch (e) { depth = e.name; }\n\
             print(r.join(), depth, mkd(D, 3).d.d.d.d);",
        ),
    },
    Case {
        path: "construct-site",
        name: "a long list of constructed cells across collections",
        src: "function N(v, next) { this.v = v; this.next = next; }\n\
              function mk(C, a, b) { return new C(a, b); }\n\
              var l = null; for (var i = 0; i < 50000; i++) l = mk(N, i, l);\n\
              var s = 0, n = 0; for (var p = l; p; p = p.next) { s += p.v; n++; }\n\
              print(n, s, l.v, Object.keys(l).join());",
        spec: Some(
            "function N(v, next) { this.v = v; this.next = next; }\n\
             function mk(C, a, b) { return new C(a, b); } for (var k = 0; k < 20; k++) mk(function D() {});\n\
             var l = null; for (var i = 0; i < 50000; i++) l = mk(N, i, l);\n\
             var s = 0, n = 0; for (var p = l; p; p = p.next) { s += p.v; n++; }\n\
             print(n, s, l.v, Object.keys(l).join());",
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
        path: "ic-named",
        name: "the smi handler kinds of a named load (in-object and out-of-line fields, array and string length) or Reflect.get",
        src: "function fresh(n) { var o = {}; for (var i = 0; i < n; i++) o['p' + i] = i; o.x = n; return o; }\n\
              function gx(o) { return o.x; } function len(o) { return o.length; }\n\
              var s = 0, r = [];\n\
              for (var k = 0; k < 3; k++) for (var n = 0; n < 14; n++) s += gx(fresh(n));\n\
              var a = [1, 2, 3], t = 'abcd';\n\
              for (var k = 0; k < 20; k++) { a.push(k); r.push(len(a), len(t + k), len(fresh(k & 3))); }\n\
              print(s, r.join());",
        spec: Some(
            "function fresh(n) { var o = {}; for (var i = 0; i < n; i++) o['p' + i] = i; o.x = n; return o; }\n\
             function gx(o) { return Reflect.get(Object(o), 'x', o); } function len(o) { return Reflect.get(Object(o), 'length', o); }\n\
             var s = 0, r = [];\n\
             for (var k = 0; k < 3; k++) for (var n = 0; n < 14; n++) s += gx(fresh(n));\n\
             var a = [1, 2, 3], t = 'abcd';\n\
             for (var k = 0; k < 20; k++) { a.push(k); r.push(len(a), len(t + k), len(fresh(k & 3))); }\n\
             print(s, r.join());",
        ),
    },
    // M8-P0 TS-STORE: a megamorphic 0x24 store probes the stub cache in the handler's cold
    // body (`ic::handler_mega_store`), and an add that needs a larger SLOTS cell grows it on the
    // miss path and stores through its case (`ic::grow_add`). The spec form stores with
    // Reflect.set (the generic [[Set]]); a strict failure throws a TypeError in both forms.
    Case {
        path: "ic-named",
        name: "an add with growth of the SLOTS cell (monomorphic, polymorphic and megamorphic store sites; the store probe of the SetNamedProperty handler) or Reflect.set",
        src: "function fresh(n) { var o = {}; for (var i = 0; i < n; i++) o['p' + i] = i; return o; }\n\
              function addx(o, v) { o.x = v; return o; }\n\
              function addyz(o, v) { o.y = v; o.z = v + 1; return o; }\n\
              function addw(o, v) { o.w = v; return o; }\n\
              var r = [], s = 0;\n\
              for (var n = 0; n < 20; n++) for (var k = 0; k < 3; k++) { var o = addyz(addx(fresh(n), n * 10 + k), k); s += o.x + o.y + o.z + Object.keys(o).length; if (k === 2 && n % 5 === 0) r.push(Object.keys(o).join()); }\n\
              for (var k = 0; k < 16; k++) { s += addw(fresh(4), k).w + addw(fresh(8 + (k & 1) * 4), k).w; }\n\
              var m = []; for (var k = 0; k < 12; k++) { var o = fresh(6); addx(o, k); addyz(o, k); m.push(o.x + o.y + o.z); }\n\
              print(s, m.join(), r.join(' | '));",
        spec: Some(
            "function fresh(n) { var o = {}; for (var i = 0; i < n; i++) o['p' + i] = i; return o; }\n\
             function addx(o, v) { Reflect.set(o, 'x', v); return o; }\n\
             function addyz(o, v) { Reflect.set(o, 'y', v); Reflect.set(o, 'z', v + 1); return o; }\n\
             function addw(o, v) { Reflect.set(o, 'w', v); return o; }\n\
             var r = [], s = 0;\n\
             for (var n = 0; n < 20; n++) for (var k = 0; k < 3; k++) { var o = addyz(addx(fresh(n), n * 10 + k), k); s += o.x + o.y + o.z + Object.keys(o).length; if (k === 2 && n % 5 === 0) r.push(Object.keys(o).join()); }\n\
             for (var k = 0; k < 16; k++) { s += addw(fresh(4), k).w + addw(fresh(8 + (k & 1) * 4), k).w; }\n\
             var m = []; for (var k = 0; k < 12; k++) { var o = fresh(6); addx(o, k); addyz(o, k); m.push(o.x + o.y + o.z); }\n\
             print(s, m.join(), r.join(' | '));",
        ),
    },
    Case {
        path: "ic-named",
        name: "an add to a frozen, sealed, non-extensible or dictionary receiver, after a setter or a read-only property appears on the prototype, and with a Proxy on the chain, or Reflect.set",
        src: "function fresh(n, p) { var o = p === undefined ? {} : Object.create(p); for (var i = 0; i < n; i++) o['p' + i] = i; return o; }\n\
              function addx(o, v) { o.x = v; return o.x; }\n\
              function saddx(o, v) { 'use strict'; o.x = v; return o.x; }\n\
              var r = [], log = [];\n\
              for (var n = 0; n < 9; n++) for (var k = 0; k < 2; k++) { r.push(addx(fresh(n), k)); r.push(saddx(fresh(n), k)); }\n\
              var f = Object.freeze(fresh(4)), se = Object.seal(fresh(4)), ne = Object.preventExtensions(fresh(4));\n\
              r.push(addx(f, 1), addx(se, 2), addx(ne, 3));\n\
              [f, se, ne].forEach(function (o) { try { saddx(o, 4); } catch (e) { r.push(e.name); } });\n\
              var d = fresh(6); delete d.p2; r.push(addx(d, 5), saddx(fresh(6), 6), Object.keys(d).join());\n\
              var P = {}, os = [];\n\
              for (var n = 0; n < 9; n++) os.push(fresh(n, P));\n\
              for (var k = 0; k < 2; k++) os.forEach(function (o, i) { r.push(addx(fresh(i, P), k)); });\n\
              Object.defineProperty(P, 'x', { set: function (v) { log.push('set' + v); }, get: function () { return 'g'; }, configurable: true });\n\
              os.forEach(function (o, i) { r.push(addx(o, i), saddx(fresh(i, P), i)); });\n\
              Object.defineProperty(P, 'x', { value: 'ro', writable: false, configurable: true });\n\
              os.forEach(function (o, i) { r.push(addx(fresh(i, P), i)); try { saddx(fresh(i, P), i); } catch (e) { r.push(e.name); } });\n\
              delete P.x;\n\
              var px = new Proxy({}, { set: function (t, k, v, rcv) { log.push('px' + String(k) + v); return Reflect.set(t, k, v, rcv); } });\n\
              for (var n = 0; n < 9; n++) { var c = fresh(0, px); c.q = n; r.push(addx(c, n), saddx(c, n + 1), Object.keys(c).join()); }\n\
              print(r.join(), log.join());",
        spec: Some(
            "function fresh(n, p) { var o = p === undefined ? {} : Object.create(p); for (var i = 0; i < n; i++) o['p' + i] = i; return o; }\n\
             function addx(o, v) { Reflect.set(o, 'x', v); return o.x; }\n\
             function saddx(o, v) { 'use strict'; if (!Reflect.set(o, 'x', v)) throw new TypeError('x'); return o.x; }\n\
             var r = [], log = [];\n\
             for (var n = 0; n < 9; n++) for (var k = 0; k < 2; k++) { r.push(addx(fresh(n), k)); r.push(saddx(fresh(n), k)); }\n\
             var f = Object.freeze(fresh(4)), se = Object.seal(fresh(4)), ne = Object.preventExtensions(fresh(4));\n\
             r.push(addx(f, 1), addx(se, 2), addx(ne, 3));\n\
             [f, se, ne].forEach(function (o) { try { saddx(o, 4); } catch (e) { r.push(e.name); } });\n\
             var d = fresh(6); delete d.p2; r.push(addx(d, 5), saddx(fresh(6), 6), Object.keys(d).join());\n\
             var P = {}, os = [];\n\
             for (var n = 0; n < 9; n++) os.push(fresh(n, P));\n\
             for (var k = 0; k < 2; k++) os.forEach(function (o, i) { r.push(addx(fresh(i, P), k)); });\n\
             Object.defineProperty(P, 'x', { set: function (v) { log.push('set' + v); }, get: function () { return 'g'; }, configurable: true });\n\
             os.forEach(function (o, i) { r.push(addx(o, i), saddx(fresh(i, P), i)); });\n\
             Object.defineProperty(P, 'x', { value: 'ro', writable: false, configurable: true });\n\
             os.forEach(function (o, i) { r.push(addx(fresh(i, P), i)); try { saddx(fresh(i, P), i); } catch (e) { r.push(e.name); } });\n\
             delete P.x;\n\
             var px = new Proxy({}, { set: function (t, k, v, rcv) { log.push('px' + String(k) + v); return Reflect.set(t, k, v, rcv); } });\n\
             for (var n = 0; n < 9; n++) { var c = fresh(0, px); c.q = n; r.push(addx(c, n), saddx(c, n + 1), Object.keys(c).join()); }\n\
             print(r.join(), log.join());",
        ),
    },
    // M8-P0 TS-STORE, the 0x25 form (JIT review B1): class fields and the properties of an
    // object literal with a `__proto__` are DefineNamedOwnProperty (0x25) adds. They need a
    // larger SLOTS cell, so they grow on the miss path (`ic::grow_add` without the validity
    // cell check): the literal at monomorphic sites (`ic::grow_store`), the fields of the second
    // pass at megamorphic sites (`ic::mega_store`, after the probe in the handler cold body).
    // A define skips the prototype chain, so the setter `x` on the prototype must not run. The
    // spec form defines with Reflect.defineProperty (CreateDataProperty). node v26 gives the
    // same line for both forms: 6,3,...,6,14 twice, then 9,0,...,9,11, and an empty log.
    Case {
        path: "ic-named",
        name: "a define with growth of the SLOTS cell (class fields and an object literal; the store probe of the DefineNamedOwnProperty handler) past a setter on the prototype, or Reflect.defineProperty",
        src: "var log = [], r = []; var P = {};\n\
              Object.defineProperty(P, 'x', { set: function (v) { log.push('set' + v); }, configurable: true });\n\
              class A { constructor(n) { for (var i = 0; i < n; i++) this['q' + i] = i; } }\n\
              class B extends A { x = 1; y = 2; z = 3; constructor(n) { super(n); } }\n\
              Object.setPrototypeOf(B.prototype, Object.create(A.prototype, { x: { set: function (v) { log.push('bset' + v); }, configurable: true } }));\n\
              for (var k = 0; k < 2; k++) for (var n = 0; n < 12; n++) { var b = new B(n); r.push(b.x + b.y + b.z, Object.keys(b).length); }\n\
              for (var n = 0; n < 12; n++) { var l = { __proto__: P, a: n, b: n + 1, c: n + 2, d: n + 3, e: n + 4, f: n + 5, g: n + 6, h: n + 7, x: n }; r.push(Object.keys(l).length, l.x); }\n\
              print(r.join(), log.join());",
        spec: Some(
            "function dp(o, k, v) { Reflect.defineProperty(o, k, { value: v, writable: true, enumerable: true, configurable: true }); }\n\
             var log = [], r = []; var P = {};\n\
             Object.defineProperty(P, 'x', { set: function (v) { log.push('set' + v); }, configurable: true });\n\
             class A { constructor(n) { for (var i = 0; i < n; i++) this['q' + i] = i; } }\n\
             class B extends A { constructor(n) { super(n); dp(this, 'x', 1); dp(this, 'y', 2); dp(this, 'z', 3); } }\n\
             Object.setPrototypeOf(B.prototype, Object.create(A.prototype, { x: { set: function (v) { log.push('bset' + v); }, configurable: true } }));\n\
             for (var k = 0; k < 2; k++) for (var n = 0; n < 12; n++) { var b = new B(n); r.push(b.x + b.y + b.z, Object.keys(b).length); }\n\
             for (var n = 0; n < 12; n++) { var l = Object.create(P); dp(l, 'a', n); dp(l, 'b', n + 1); dp(l, 'c', n + 2); dp(l, 'd', n + 3); dp(l, 'e', n + 4); dp(l, 'f', n + 5); dp(l, 'g', n + 6); dp(l, 'h', n + 7); dp(l, 'x', n); r.push(Object.keys(l).length, l.x); }\n\
             print(r.join(), log.join());",
        ),
    },
    // ---- ic-global (M7-P2 GLOBALS) ----
    // Each form runs its harness (the counters, the loops, the called functions) in a function
    // scope, so only the binding under test is global, and the functions that read or write it
    // run 12 or more times: past the feedback threshold (8 calls), so the src form reaches the
    // global IC hit path, and the spec form (Reflect on globalThis, or a with-scope lookup)
    // does not hit the global IC (fixer r2b: Claude reviewer REV-5, JIT reviewer J5).
    Case {
        path: "ic-global",
        name: "a var, a function and a builtin: IC hits or Reflect on globalThis",
        src: "var g = 1; function inc() { g = g + 1; return g; }\n\
              (function () {\n\
                function rd() { return g + (typeof Math).length + parseInt('7'); }\n\
                function step() { inc(); return rd(); }\n\
                var s = 0; for (var i = 0; i < 30; i++) s += step();\n\
                print(s, g, typeof notThere);\n\
              })();",
        spec: Some(
            "var g = 1, inc;\n\
             (function () {\n\
               var R = Reflect, G = globalThis;\n\
               R.set(G, 'inc', function () { R.set(G, 'g', R.get(G, 'g') + 1); return R.get(G, 'g'); });\n\
               function rd() { return R.get(G, 'g') + (typeof R.get(G, 'Math')).length + R.get(G, 'parseInt')('7'); }\n\
               function step() { R.get(G, 'inc')(); return rd(); }\n\
               var s = 0; for (var i = 0; i < 30; i++) s += step();\n\
               print(s, R.get(G, 'g'), typeof R.get(G, 'notThere'));\n\
             })();",
        ),
    },
    Case {
        path: "ic-global",
        name: "the global object's shape changes between hits (polymorphic), then deletes",
        src: "var g = 3;\n\
              (function () {\n\
                function f() { return typeof h1 === 'undefined' ? g : g + h1; }\n\
                var G = globalThis, s = 0; for (var r = 0; r < 12; r++) s += f();\n\
                G.h1 = 1; s += f() + f(); G.h2 = 2; s += f() + f();\n\
                G.h3 = 3; s += f() + f(); g = 10; s += f();\n\
                delete G.h1; s += f() + f(); delete G.h3; s += f() + f();\n\
                print(s, typeof h1, typeof h2, typeof h3);\n\
              })();",
        spec: Some(
            "var g = 3;\n\
             (function () {\n\
               var R = Reflect, G = globalThis;\n\
               function f() { return R.has(G, 'h1') ? R.get(G, 'g') + R.get(G, 'h1') : R.get(G, 'g'); }\n\
               var s = 0; for (var r = 0; r < 12; r++) s += f();\n\
               G.h1 = 1; s += f() + f(); G.h2 = 2; s += f() + f();\n\
               G.h3 = 3; s += f() + f(); R.set(G, 'g', 10); s += f();\n\
               delete G.h1; s += f() + f(); delete G.h3; s += f() + f();\n\
               print(s, typeof R.get(G, 'h1'), typeof R.get(G, 'h2'), typeof R.get(G, 'h3'));\n\
             })();",
        ),
    },
    Case {
        path: "ic-global",
        name: "let and const bindings and the TDZ: IC hits or a with-scope lookup",
        src: "(function () {\n\
                function early() { return T; }\n\
                var e = 0; for (var i = 0; i < 12; i++) { try { early(); } catch (x) { if (x.name === 'ReferenceError') e++; } }\n\
                print(e);\n\
              })();\n\
              let T = 4; let L = 2; const C = 5;\n\
              (function () {\n\
                function early() { return T; } function f() { L = L + C; return L; } function setC() { C = 1; }\n\
                var s = 0; for (var i = 0; i < 20; i++) s += f() + early();\n\
                var e = 0; for (var i = 0; i < 12; i++) { try { setC(); } catch (x) { if (x.name === 'TypeError') e++; } }\n\
                print(s, e, L, C, typeof T);\n\
              })();",
        spec: Some(
            "(function () {\n\
               function early() { with ({}) return T; }\n\
               var e = 0; for (var i = 0; i < 12; i++) { try { early(); } catch (x) { if (x.name === 'ReferenceError') e++; } }\n\
               print(e);\n\
             })();\n\
             let T = 4; let L = 2; const C = 5;\n\
             (function () {\n\
               function early() { with ({}) return T; } function f() { with ({}) { L = L + C; return L; } }\n\
               function setC() { with ({}) C = 1; }\n\
               var s = 0; for (var i = 0; i < 20; i++) s += f() + early();\n\
               var e = 0; for (var i = 0; i < 12; i++) { try { setC(); } catch (x) { if (x.name === 'TypeError') e++; } }\n\
               with ({}) print(s, e, L, C, typeof T);\n\
             })();",
        ),
    },
    Case {
        path: "ic-global",
        name: "a lazy namespace key (Temporal): the first load installs it, later loads hit",
        src: "(function () {\n\
                function t() { return typeof Temporal; } function g() { return Temporal; }\n\
                var a, b, c = 0; for (var i = 0; i < 20; i++) { a = g(); b = t(); if (a === g()) c++; }\n\
                print(b, Object.prototype.toString.call(a), c, a === globalThis.Temporal);\n\
                function w(v) { Temporal = v; } for (var i = 0; i < 20; i++) w(i);\n\
                print(g(), t());\n\
              })();",
        spec: Some(
            "(function () {\n\
               var R = Reflect, G = globalThis;\n\
               function t() { return typeof R.get(G, 'Temporal'); } function g() { return R.get(G, 'Temporal'); }\n\
               var a, b, c = 0; for (var i = 0; i < 20; i++) { a = g(); b = t(); if (a === g()) c++; }\n\
               print(b, Object.prototype.toString.call(a), c, a === G.Temporal);\n\
               function w(v) { R.set(G, 'Temporal', v); } for (var i = 0; i < 20; i++) w(i);\n\
               print(g(), t());\n\
             })();",
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
    // The ELEMKINDS levels (M7-P3 fixer r1): `keyed_ic::fast_set` sends a FROZEN array cold
    // in each state of the site (no vector, monomorphic, megamorphic); a SEALED one stores.
    Case {
        path: "keyed-access",
        name: "a frozen or sealed array at a store site",
        src: "function once(a) { a[0] = 9; return a[0]; } print(once(Object.freeze([1, 2])));\n\
              function st(a, i, v) { a[i] = v; return a[i]; } function sst(a, i, v) { 'use strict'; a[i] = v; return a[i]; }\n\
              var p = [1, 2, 3]; for (var i = 0; i < 20; i++) { st(p, i % 3, i); sst(p, i % 3, i); }\n\
              var f = Object.freeze([1, 2, 3]), s = Object.seal([1.5, 2.5]);\n\
              print(st(f, 0, 9), st(s, 1, 0.25), st(p, 2, 7), sst(s, 0, 'x'));\n\
              try { sst(f, 1, 9); } catch (e) { print(e.name); }\n\
              print(f.join(), s.join(), p.join(), Object.isFrozen(f), Object.isSealed(s), Object.isFrozen(s));",
        spec: Some(
            "function once(a) { a[String(0)] = 9; return a[String(0)]; } print(once(Object.freeze([1, 2])));\n\
             function st(a, i, v) { a[String(i)] = v; return a[String(i)]; } function sst(a, i, v) { 'use strict'; a[String(i)] = v; return a[String(i)]; }\n\
             var p = [1, 2, 3]; for (var i = 0; i < 20; i++) { st(p, i % 3, i); sst(p, i % 3, i); }\n\
             var f = Object.freeze([1, 2, 3]), s = Object.seal([1.5, 2.5]);\n\
             print(st(f, 0, 9), st(s, 1, 0.25), st(p, 2, 7), sst(s, 0, 'x'));\n\
             try { sst(f, 1, 9); } catch (e) { print(e.name); }\n\
             print(f.join(), s.join(), p.join(), Object.isFrozen(f), Object.isSealed(s), Object.isFrozen(s));",
        ),
    },
    Case {
        path: "keyed-access",
        name: "a frozen array at a megamorphic store site",
        src: "function st(a, v) { a[0] = v; return a[0]; } function sst(a, v) { 'use strict'; a[0] = v; return a[0]; }\n\
              var m = []; for (var j = 0; j < 8; j++) { var x = j % 2 ? [j + 0.5, j] : [j, j]; x['k' + j] = j; m.push(x); }\n\
              for (var r = 0; r < 4; r++) for (var j = 0; j < 8; j++) { st(m[j], r); sst(m[j], r); }\n\
              var g = Object.freeze([4, 5]), h = Object.freeze([4.5, 5]), q = Object.seal(['a', 'b']);\n\
              print(st(g, 1), st(h, 2), st(q, 3), sst(q, 4), g.join(), h.join(), q.join());\n\
              try { sst(g, 1); } catch (e) { print(e.name); } try { sst(h, 1); } catch (e) { print(e.name); }\n\
              print(m.map(function (a) { return a[0]; }).join());",
        spec: Some(
            "function st(a, v) { a[String(0)] = v; return a[String(0)]; } function sst(a, v) { 'use strict'; a[String(0)] = v; return a[String(0)]; }\n\
             var m = []; for (var j = 0; j < 8; j++) { var x = j % 2 ? [j + 0.5, j] : [j, j]; x['k' + j] = j; m.push(x); }\n\
             for (var r = 0; r < 4; r++) for (var j = 0; j < 8; j++) { st(m[j], r); sst(m[j], r); }\n\
             var g = Object.freeze([4, 5]), h = Object.freeze([4.5, 5]), q = Object.seal(['a', 'b']);\n\
             print(st(g, 1), st(h, 2), st(q, 3), sst(q, 4), g.join(), h.join(), q.join());\n\
             try { sst(g, 1); } catch (e) { print(e.name); } try { sst(h, 1); } catch (e) { print(e.name); }\n\
             print(m.map(function (a) { return a[0]; }).join());",
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
    // ---- the T0 forward (M7-P4 LEVER-3: `call::forward_frame` in the T0 call fast path;
    // each loop runs past the feedback threshold, so both the T0 form and the cold path run) ----
    Case {
        path: "forward-apply",
        name: "the T0 forward: unmapped arguments in a loop",
        src: "function u(a, b) { 'use strict'; return show.apply(this, arguments); }\n\
              function w() { 'use strict'; return show.apply(undefined, arguments); }\n\
              function st() { 'use strict'; return [this === undefined ? 'u' : typeof this, arguments.length].join(); }\n\
              function s5() { 'use strict'; return st.apply(5, arguments); }\n\
              var o = { u: u }, r = [];\n\
              for (var i = 0; i < 300; i++) r.push(o.u(i, 2, 3), u(), w(i), s5(i, i), st.apply(undefined, [i]));\n\
              print(r.length, r.slice(0, 5).join(' / '), r.slice(-5).join(' / '));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "the T0 forward: packed Arrays in a loop",
        src: "var a0 = [], a1 = ['x'], a3 = [1, 'two', {}], d = [1.5, 2.5], big = [], r = [];\n\
              for (var j = 0; j < 40; j++) big.push(j);\n\
              function st() { 'use strict'; return [typeof this, arguments.length, arguments[2]].join(); }\n\
              for (var i = 0; i < 300; i++) {\n\
                r.push(show.apply(null, a0), show.apply(a1, a1), show.apply(1, a3), show.apply('s', d), show.apply(null, big), st.apply(1, a3));\n\
                if (i == 150) { a3.length = 2; a1.push('y'); d[5] = 1; big[2] = 0.5; }\n\
                if (i == 200) { delete a1[0]; Array.prototype[0] = 'P'; }\n\
              }\n\
              delete Array.prototype[0];\n\
              print(r.length, r.slice(0, 6).join(' / '), r.slice(-6).join(' / '));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "the T0 forward: a mapped object and a changed list",
        src: "function m(a, b) { return show.apply(this, arguments); }\n\
              function u(a, b) { 'use strict'; arguments[0] = 'U'; if (a === 3) arguments.length = 1; return show.apply(this, arguments); }\n\
              function x() { 'use strict'; arguments[5] = 'far'; return show.apply(null, arguments); }\n\
              function y() { 'use strict'; delete arguments[0]; return show.apply(null, arguments); }\n\
              var r = [];\n\
              for (var i = 0; i < 200; i++) r.push(m(i, 1), u(i, 2), u(3, 4), x(1), y(1, 2));\n\
              print(r.length, r.slice(0, 5).join(' / '), r.slice(-5).join(' / '));",
        spec: None,
    },
    Case {
        path: "forward-call",
        name: "the T0 forward: call in a loop",
        src: "function st() { 'use strict'; return [this === undefined ? 'u' : typeof this, arguments.length, Array.prototype.join.call(arguments)].join(':'); }\n\
              function two(a, b) { return a + b; }\n\
              var o = { m: show }, r = [];\n\
              for (var i = 0; i < 300; i++) r.push(show.call(), show.call(o), show.call(null, i), show.call(o, 1, 2),\n\
                show.call(o, 1, 2, 3, 4, 5), st.call(i, 'a'), st.call(undefined), two.call(null, i), two.call(null, i, 1, 2));\n\
              print(r.length, r.slice(0, 9).join(' / '), r.slice(-9).join(' / '));",
        spec: None,
    },
    Case {
        path: "forward-apply",
        name: "the T0 forward: a stack overflow of apply and call",
        src: "function deep(n) { return deep.apply(null, [n + 1]); }\n\
              function deepc(n) { return deepc.call(null, n + 1); }\n\
              function deepa() { 'use strict'; return deepa.apply(null, arguments); }\n\
              try { deep(0); } catch (e) { print(e.name, e.message); }\n\
              try { deepc(0); } catch (e) { print(e.name, e.message); }\n\
              try { deepa(1, 2); } catch (e) { print(e.name, e.message); }\n\
              print(show.apply(null, [1, 2]), show.call(null, 3));",
        spec: None,
    },
    // M7-P4 LEVER-2 (TSPASS2): the IC hit of a prefixed site. `f` loads 300 other names
    // first, so the name constant of `o.x` needs a `Wide` operand, and its loop body is longer
    // than 255 bytes (`Wide` JumpIfFalse, JumpIfToBooleanFalse and JumpLoop): on aarch64 the
    // `Wide` fast body runs them (`interp/ops.rs` `prefix_runs_fast`). Mono, falsy, a string
    // ToBoolean (cold), a getter (cold), 8 shapes (megamorphic), a prototype load and its change.
    Case {
        path: "ic-named",
        name: "a prefixed (Wide) IC hit or Reflect.get",
        src: "function f(o, n) {\n\
              o.a0; o.a1; o.a2; o.a3; o.a4; o.a5; o.a6; o.a7; o.a8; o.a9;\n\
              o.a10; o.a11; o.a12; o.a13; o.a14; o.a15; o.a16; o.a17; o.a18; o.a19;\n\
              o.a20; o.a21; o.a22; o.a23; o.a24; o.a25; o.a26; o.a27; o.a28; o.a29;\n\
              o.a30; o.a31; o.a32; o.a33; o.a34; o.a35; o.a36; o.a37; o.a38; o.a39;\n\
              o.a40; o.a41; o.a42; o.a43; o.a44; o.a45; o.a46; o.a47; o.a48; o.a49;\n\
              o.a50; o.a51; o.a52; o.a53; o.a54; o.a55; o.a56; o.a57; o.a58; o.a59;\n\
              o.a60; o.a61; o.a62; o.a63; o.a64; o.a65; o.a66; o.a67; o.a68; o.a69;\n\
              o.a70; o.a71; o.a72; o.a73; o.a74; o.a75; o.a76; o.a77; o.a78; o.a79;\n\
              o.a80; o.a81; o.a82; o.a83; o.a84; o.a85; o.a86; o.a87; o.a88; o.a89;\n\
              o.a90; o.a91; o.a92; o.a93; o.a94; o.a95; o.a96; o.a97; o.a98; o.a99;\n\
              o.a100; o.a101; o.a102; o.a103; o.a104; o.a105; o.a106; o.a107; o.a108; o.a109;\n\
              o.a110; o.a111; o.a112; o.a113; o.a114; o.a115; o.a116; o.a117; o.a118; o.a119;\n\
              o.a120; o.a121; o.a122; o.a123; o.a124; o.a125; o.a126; o.a127; o.a128; o.a129;\n\
              o.a130; o.a131; o.a132; o.a133; o.a134; o.a135; o.a136; o.a137; o.a138; o.a139;\n\
              o.a140; o.a141; o.a142; o.a143; o.a144; o.a145; o.a146; o.a147; o.a148; o.a149;\n\
              o.a150; o.a151; o.a152; o.a153; o.a154; o.a155; o.a156; o.a157; o.a158; o.a159;\n\
              o.a160; o.a161; o.a162; o.a163; o.a164; o.a165; o.a166; o.a167; o.a168; o.a169;\n\
              o.a170; o.a171; o.a172; o.a173; o.a174; o.a175; o.a176; o.a177; o.a178; o.a179;\n\
              o.a180; o.a181; o.a182; o.a183; o.a184; o.a185; o.a186; o.a187; o.a188; o.a189;\n\
              o.a190; o.a191; o.a192; o.a193; o.a194; o.a195; o.a196; o.a197; o.a198; o.a199;\n\
              o.a200; o.a201; o.a202; o.a203; o.a204; o.a205; o.a206; o.a207; o.a208; o.a209;\n\
              o.a210; o.a211; o.a212; o.a213; o.a214; o.a215; o.a216; o.a217; o.a218; o.a219;\n\
              o.a220; o.a221; o.a222; o.a223; o.a224; o.a225; o.a226; o.a227; o.a228; o.a229;\n\
              o.a230; o.a231; o.a232; o.a233; o.a234; o.a235; o.a236; o.a237; o.a238; o.a239;\n\
              o.a240; o.a241; o.a242; o.a243; o.a244; o.a245; o.a246; o.a247; o.a248; o.a249;\n\
              o.a250; o.a251; o.a252; o.a253; o.a254; o.a255; o.a256; o.a257; o.a258; o.a259;\n\
              o.a260; o.a261; o.a262; o.a263; o.a264; o.a265; o.a266; o.a267; o.a268; o.a269;\n\
              o.a270; o.a271; o.a272; o.a273; o.a274; o.a275; o.a276; o.a277; o.a278; o.a279;\n\
              o.a280; o.a281; o.a282; o.a283; o.a284; o.a285; o.a286; o.a287; o.a288; o.a289;\n\
              o.a290; o.a291; o.a292; o.a293; o.a294; o.a295; o.a296; o.a297; o.a298; o.a299;\n\
              var s = 0;\n\
              for (var i = 0; i < n; i++) {\n\
              var t = o.x;\n\
              if (t) {\n\
              s += t;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
              }\n\
              }\n\
              return s;\n\
              }\n\
              var r = [f({ x: 2 }, 50), f({ x: 0 }, 5), f({ x: 'ab' }, 3)];\n\
              var g = { get x() { return 7; } }; r.push(f(g, 4));\n\
              for (var j = 0; j < 8; j++) { var o = {}; o['p' + j] = j; o.x = j + 1; r.push(f(o, 3)); }\n\
              var P = { x: 100 }; var c = Object.create(P); r.push(f(c, 2)); P.x = 5; r.push(f(c, 2));\n\
              print(r.join(' '));",
        spec: Some(
            "function f(o, n) {\n\
             o.a0; o.a1; o.a2; o.a3; o.a4; o.a5; o.a6; o.a7; o.a8; o.a9;\n\
             o.a10; o.a11; o.a12; o.a13; o.a14; o.a15; o.a16; o.a17; o.a18; o.a19;\n\
             o.a20; o.a21; o.a22; o.a23; o.a24; o.a25; o.a26; o.a27; o.a28; o.a29;\n\
             o.a30; o.a31; o.a32; o.a33; o.a34; o.a35; o.a36; o.a37; o.a38; o.a39;\n\
             o.a40; o.a41; o.a42; o.a43; o.a44; o.a45; o.a46; o.a47; o.a48; o.a49;\n\
             o.a50; o.a51; o.a52; o.a53; o.a54; o.a55; o.a56; o.a57; o.a58; o.a59;\n\
             o.a60; o.a61; o.a62; o.a63; o.a64; o.a65; o.a66; o.a67; o.a68; o.a69;\n\
             o.a70; o.a71; o.a72; o.a73; o.a74; o.a75; o.a76; o.a77; o.a78; o.a79;\n\
             o.a80; o.a81; o.a82; o.a83; o.a84; o.a85; o.a86; o.a87; o.a88; o.a89;\n\
             o.a90; o.a91; o.a92; o.a93; o.a94; o.a95; o.a96; o.a97; o.a98; o.a99;\n\
             o.a100; o.a101; o.a102; o.a103; o.a104; o.a105; o.a106; o.a107; o.a108; o.a109;\n\
             o.a110; o.a111; o.a112; o.a113; o.a114; o.a115; o.a116; o.a117; o.a118; o.a119;\n\
             o.a120; o.a121; o.a122; o.a123; o.a124; o.a125; o.a126; o.a127; o.a128; o.a129;\n\
             o.a130; o.a131; o.a132; o.a133; o.a134; o.a135; o.a136; o.a137; o.a138; o.a139;\n\
             o.a140; o.a141; o.a142; o.a143; o.a144; o.a145; o.a146; o.a147; o.a148; o.a149;\n\
             o.a150; o.a151; o.a152; o.a153; o.a154; o.a155; o.a156; o.a157; o.a158; o.a159;\n\
             o.a160; o.a161; o.a162; o.a163; o.a164; o.a165; o.a166; o.a167; o.a168; o.a169;\n\
             o.a170; o.a171; o.a172; o.a173; o.a174; o.a175; o.a176; o.a177; o.a178; o.a179;\n\
             o.a180; o.a181; o.a182; o.a183; o.a184; o.a185; o.a186; o.a187; o.a188; o.a189;\n\
             o.a190; o.a191; o.a192; o.a193; o.a194; o.a195; o.a196; o.a197; o.a198; o.a199;\n\
             o.a200; o.a201; o.a202; o.a203; o.a204; o.a205; o.a206; o.a207; o.a208; o.a209;\n\
             o.a210; o.a211; o.a212; o.a213; o.a214; o.a215; o.a216; o.a217; o.a218; o.a219;\n\
             o.a220; o.a221; o.a222; o.a223; o.a224; o.a225; o.a226; o.a227; o.a228; o.a229;\n\
             o.a230; o.a231; o.a232; o.a233; o.a234; o.a235; o.a236; o.a237; o.a238; o.a239;\n\
             o.a240; o.a241; o.a242; o.a243; o.a244; o.a245; o.a246; o.a247; o.a248; o.a249;\n\
             o.a250; o.a251; o.a252; o.a253; o.a254; o.a255; o.a256; o.a257; o.a258; o.a259;\n\
             o.a260; o.a261; o.a262; o.a263; o.a264; o.a265; o.a266; o.a267; o.a268; o.a269;\n\
             o.a270; o.a271; o.a272; o.a273; o.a274; o.a275; o.a276; o.a277; o.a278; o.a279;\n\
             o.a280; o.a281; o.a282; o.a283; o.a284; o.a285; o.a286; o.a287; o.a288; o.a289;\n\
             o.a290; o.a291; o.a292; o.a293; o.a294; o.a295; o.a296; o.a297; o.a298; o.a299;\n\
             var s = 0;\n\
             for (var i = 0; i < n; i++) {\n\
             var t = Reflect.get(o, 'x');\n\
             if (t) {\n\
             s += t;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0; i |= 0;\n\
             }\n\
             }\n\
             return s;\n\
             }\n\
             var r = [f({ x: 2 }, 50), f({ x: 0 }, 5), f({ x: 'ab' }, 3)];\n\
             var g = { get x() { return 7; } }; r.push(f(g, 4));\n\
             for (var j = 0; j < 8; j++) { var o = {}; o['p' + j] = j; o.x = j + 1; r.push(f(o, 3)); }\n\
             var P = { x: 100 }; var c = Object.create(P); r.push(f(c, 2)); P.x = 5; r.push(f(c, 2));\n\
             print(r.join(' '));",
        ),
    },
    // M7-P5 LEVER-4 (ICWIDE): `keyed_ic::fast_set` stores a missing element of an Array in
    // place (`grow_in_place`): at or past `length` and into a hole, inside the capacity, with
    // no kind transition, on an extensible array that is not a prototype, while no object on
    // the chain has an index key. The spec form names each index by a string key (cold).
    Case {
        path: "keyed-access",
        name: "a store at or past the length or into a hole of an Array, in place",
        src: "function put(a, i, v) { a[i] = v; return a.length; }\n\
              var p = [], d = [0.5], o = ['x'], h = new Array(6), r = [];\n\
              for (var i = 0; i < 20; i++) { put(p, p.length, i); put(d, d.length, i + 0.25); put(o, o.length, {k: i}); }\n\
              for (var i = 5; i >= 0; i -= 2) r.push(put(h, i, i * 3));\n\
              for (var i = 0; i < 6; i += 2) r.push(put(h, i, -i));\n\
              r.push(put(p, 25, 1), put(p, 21, 2), put(p, 20, 3), put(d, d.length, NaN), put(d, d.length, 7), put(d, d.length, 'z'));\n\
              var q = [1, 2, 3]; q.length = 1; r.push(put(q, 1, 9), put(q, 2, 8.5), put(q, 3, 'w'));\n\
              var e = []; e.length = 3; r.push(put(e, 0, 1), put(e, 3, 4));\n\
              print(r.join(), p.join(), d.join(), o.map(function (x) { return typeof x === 'object' ? x.k : x; }).join(), h.join(), q.join(), e.join());\n\
              print(20 in p, 22 in p, 24 in p, 5 in h, Object.keys(h).join(), Object.keys(e).join(), String(d[21]));",
        spec: Some(
            "function put(a, i, v) { a[String(i)] = v; return a.length; }\n\
             var p = [], d = [0.5], o = ['x'], h = new Array(6), r = [];\n\
             for (var i = 0; i < 20; i++) { put(p, p.length, i); put(d, d.length, i + 0.25); put(o, o.length, {k: i}); }\n\
             for (var i = 5; i >= 0; i -= 2) r.push(put(h, i, i * 3));\n\
             for (var i = 0; i < 6; i += 2) r.push(put(h, i, -i));\n\
             r.push(put(p, 25, 1), put(p, 21, 2), put(p, 20, 3), put(d, d.length, NaN), put(d, d.length, 7), put(d, d.length, 'z'));\n\
             var q = [1, 2, 3]; q.length = 1; r.push(put(q, 1, 9), put(q, 2, 8.5), put(q, 3, 'w'));\n\
             var e = []; e.length = 3; r.push(put(e, 0, 1), put(e, 3, 4));\n\
             print(r.join(), p.join(), d.join(), o.map(function (x) { return typeof x === 'object' ? x.k : x; }).join(), h.join(), q.join(), e.join());\n\
             print(20 in p, 22 in p, 24 in p, 5 in h, Object.keys(h).join(), Object.keys(e).join(), String(d[21]));",
        ),
    },
    Case {
        path: "keyed-access",
        name: "a store past the length of a non-extensible, sealed or frozen Array, or of an Array that is a prototype",
        src: "function put(a, i, v) { a[i] = v; return a.length; } function sput(a, i, v) { 'use strict'; a[i] = v; return a.length; }\n\
              var w = [1, 2]; for (var i = 0; i < 3; i++) put(w, w.length, i);\n\
              var n = Object.preventExtensions([1, 2]), s = Object.seal([1, 2]), f = Object.freeze([1, 2]), r = [];\n\
              r.push(put(n, 2, 3), put(s, 2, 3), put(f, 2, 3));\n\
              try { sput(n, 2, 3); } catch (x) { r.push(x.name); } try { sput(s, 2, 3); } catch (x) { r.push(x.name); }\n\
              var pa = [1, 2]; var c = Object.create(pa); r.push(put(pa, 2, 3), put(pa, 3, 4), c[2], c[3], c.length);\n\
              var t = [1, 2]; t.length = 4; Object.defineProperty(t, 'length', { writable: false }); r.push(put(t, 4, 5), put(t, 2, 6));\n\
              try { sput(t, 4, 5); } catch (x) { r.push(x.name); }\n\
              print(r.join(), w.join(), n.join(), s.join(), f.join(), pa.join(), t.join(), t.length, Object.isExtensible(n));",
        spec: Some(
            "function put(a, i, v) { a[String(i)] = v; return a.length; } function sput(a, i, v) { 'use strict'; a[String(i)] = v; return a.length; }\n\
             var w = [1, 2]; for (var i = 0; i < 3; i++) put(w, w.length, i);\n\
             var n = Object.preventExtensions([1, 2]), s = Object.seal([1, 2]), f = Object.freeze([1, 2]), r = [];\n\
             r.push(put(n, 2, 3), put(s, 2, 3), put(f, 2, 3));\n\
             try { sput(n, 2, 3); } catch (x) { r.push(x.name); } try { sput(s, 2, 3); } catch (x) { r.push(x.name); }\n\
             var pa = [1, 2]; var c = Object.create(pa); r.push(put(pa, 2, 3), put(pa, 3, 4), c[2], c[3], c.length);\n\
             var t = [1, 2]; t.length = 4; Object.defineProperty(t, 'length', { writable: false }); r.push(put(t, 4, 5), put(t, 2, 6));\n\
             try { sput(t, 4, 5); } catch (x) { r.push(x.name); }\n\
             print(r.join(), w.join(), n.join(), s.join(), f.join(), pa.join(), t.join(), t.length, Object.isExtensible(n));",
        ),
    },
    // An index key on a prototype (the static fuse "no-elements prototypes" pops): a setter or
    // a read-only element of Array.prototype or Object.prototype decides the store of a
    // missing element (§10.1.9.2), so it stays on the generic path.
    Case {
        path: "keyed-access",
        name: "a store past the length or into a hole with an index setter or a read-only index on the chain",
        src: "function put(a, i, v) { a[i] = v; return a.length; } function sput(a, i, v) { 'use strict'; a[i] = v; return a.length; }\n\
              var log = [], a = [0, 1], h = [0, , 2], b = [0];\n\
              for (var i = 0; i < 4; i++) put(b, b.length, i);\n\
              Object.defineProperty(Array.prototype, 2, { set: function (v) { log.push('set' + v); }, configurable: true });\n\
              Object.defineProperty(Object.prototype, 5, { value: 'ro', writable: false, configurable: true });\n\
              var r = [put(a, 2, 7), put(h, 1, 8), put(b, 5, 9), put(b, b.length, 10)];\n\
              try { sput([0, 1, 2, 3, 4], 5, 1); } catch (x) { r.push(x.name); }\n\
              delete Array.prototype[2]; delete Object.prototype[5];\n\
              r.push(put(a, 2, 11), put(a, a.length, 12));\n\
              print(r.join(), log.join(), a.join(), h.join(), b.join(), 2 in a, a.hasOwnProperty(2));",
        spec: Some(
            "function put(a, i, v) { a[String(i)] = v; return a.length; } function sput(a, i, v) { 'use strict'; a[String(i)] = v; return a.length; }\n\
             var log = [], a = [0, 1], h = [0, , 2], b = [0];\n\
             for (var i = 0; i < 4; i++) put(b, b.length, i);\n\
             Object.defineProperty(Array.prototype, 2, { set: function (v) { log.push('set' + v); }, configurable: true });\n\
             Object.defineProperty(Object.prototype, 5, { value: 'ro', writable: false, configurable: true });\n\
             var r = [put(a, 2, 7), put(h, 1, 8), put(b, 5, 9), put(b, b.length, 10)];\n\
             try { sput([0, 1, 2, 3, 4], 5, 1); } catch (x) { r.push(x.name); }\n\
             delete Array.prototype[2]; delete Object.prototype[5];\n\
             r.push(put(a, 2, 11), put(a, a.length, 12));\n\
             print(r.join(), log.join(), a.join(), h.join(), b.join(), 2 in a, a.hasOwnProperty(2));",
        ),
    },
    // ---- keyed-view (TypedArray TA-2 TA-KEYED; design §6.5) ----
    // The views come from the host functions of `run_hits` (no JS constructor before TA-2
    // TA-BUFFERS): `__ab(bytes, max)`, `__sab(bytes, max)`, `__ta(type, buffer, offset,
    // length)` (a Table 71 row; null prototype), `__detach(buffer)`, `__resize(buffer, n)`,
    // `__grow(sab, n)`. The spec form names each index by a string key (CanonicalNumericIndexString:
    // the cold exotic path). Each fast form was checked against node v26.13.0 with the real
    // constructors (the TA-KEYED evidence `fastdiff-node`).
    Case {
        path: "keyed-view",
        name: "an in-bounds load and store of each element type",
        src: VIEW_TYPES_FAST,
        spec: Some(VIEW_TYPES_SPEC),
    },
    Case {
        path: "keyed-view",
        name: "an out-of-bounds or negative index, sloppy and strict",
        src: "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; } function sst(a, i, v) { 'use strict'; a[i] = v; }\n\
              function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
              var a = __ta(5, __ab(16), 0, 4), r = [];\n\
              for (var k = 0; k < 2; k++) for (var i = -2; i < 7; i++) { st(a, i, i * 10); sst(a, i, i * 10 + 1); r.push(sh(ld(a, i))); }\n\
              print(r.join());",
        spec: Some(
            "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; } function sst(a, i, v) { 'use strict'; a[String(i)] = v; }\n\
             function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
             var a = __ta(5, __ab(16), 0, 4), r = [];\n\
             for (var k = 0; k < 2; k++) for (var i = -2; i < 7; i++) { st(a, i, i * 10); sst(a, i, i * 10 + 1); r.push(sh(ld(a, i))); }\n\
             print(r.join());",
        ),
    },
    Case {
        path: "keyed-view",
        name: "a detached view, and a store whose valueOf detaches",
        src: "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; } function sst(a, i, v) { 'use strict'; a[i] = v; }\n\
              function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
              var b = __ab(16), a = __ta(5, b, 0, 4), f = __ta(0, b, 8, 8), r = [];\n\
              for (var k = 0; k < 2; k++) for (var i = 0; i < 4; i++) { st(a, i, i + 1); st(f, i, -i); r.push(ld(a, i), ld(f, i)); }\n\
              __detach(b);\n\
              for (var i = 0; i < 4; i++) { st(a, i, 9); sst(f, i, 9); r.push(sh(ld(a, i)), sh(ld(f, i))); }\n\
              var c = __ab(8), u = __ta(0, c, 0, 8), log = [];\n\
              for (var i = 0; i < 4; i++) st(u, i, i);\n\
              st(u, 1, { valueOf: function () { log.push('v'); __detach(c); return 5; } });\n\
              st(u, 9, { valueOf: function () { log.push('w'); return 1; } });\n\
              print(r.join(), log.join(), sh(ld(u, 0)), sh(ld(u, 1)));",
        spec: Some(
            "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; } function sst(a, i, v) { 'use strict'; a[String(i)] = v; }\n\
             function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
             var b = __ab(16), a = __ta(5, b, 0, 4), f = __ta(0, b, 8, 8), r = [];\n\
             for (var k = 0; k < 2; k++) for (var i = 0; i < 4; i++) { st(a, i, i + 1); st(f, i, -i); r.push(ld(a, i), ld(f, i)); }\n\
             __detach(b);\n\
             for (var i = 0; i < 4; i++) { st(a, i, 9); sst(f, i, 9); r.push(sh(ld(a, i)), sh(ld(f, i))); }\n\
             var c = __ab(8), u = __ta(0, c, 0, 8), log = [];\n\
             for (var i = 0; i < 4; i++) st(u, i, i);\n\
             st(u, 1, { valueOf: function () { log.push('v'); __detach(c); return 5; } });\n\
             st(u, 9, { valueOf: function () { log.push('w'); return 1; } });\n\
             print(r.join(), log.join(), sh(ld(u, 0)), sh(ld(u, 1)));",
        ),
    },
    Case {
        path: "keyed-view",
        name: "Uint8Clamped rounding, and NaN stores read back through a Uint8Array",
        src: "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; }\n\
              var a = __ta(2, __ab(8), 0, 8), V = [1.5, 2.5, -1, 300, NaN, 254.5, 255.5, 0.5, 7], r = [];\n\
              for (var k = 0; k < 2; k++) for (var i = 0; i < V.length; i++) { st(a, i % 8, V[i]); r.push(ld(a, i % 8)); }\n\
              var b = __ab(16), f = __ta(11, b, 0, 1), g = __ta(10, b, 8, 1), u = __ta(1, b, 0, 16), q = [];\n\
              for (var k = 0; k < 3; k++) { st(f, 0, NaN); st(g, 0, 0 / 0); }\n\
              for (var i = 0; i < 16; i++) q.push(ld(u, i));\n\
              print(r.join(), q.join(), ld(f, 0), ld(g, 0));",
        spec: Some(
            "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; }\n\
             var a = __ta(2, __ab(8), 0, 8), V = [1.5, 2.5, -1, 300, NaN, 254.5, 255.5, 0.5, 7], r = [];\n\
             for (var k = 0; k < 2; k++) for (var i = 0; i < V.length; i++) { st(a, i % 8, V[i]); r.push(ld(a, i % 8)); }\n\
             var b = __ab(16), f = __ta(11, b, 0, 1), g = __ta(10, b, 8, 1), u = __ta(1, b, 0, 16), q = [];\n\
             for (var k = 0; k < 3; k++) { st(f, 0, NaN); st(g, 0, 0 / 0); }\n\
             for (var i = 0; i < 16; i++) q.push(ld(u, i));\n\
             print(r.join(), q.join(), ld(f, 0), ld(g, 0));",
        ),
    },
    Case {
        path: "keyed-view",
        name: "a shrunk resizable buffer and an auto view after a grow",
        src: "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; }\n\
              function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
              var b = __ab(8, 16), auto = __ta(0, b, 0), fx = __ta(0, b, 4, 4), r = [];\n\
              for (var i = 0; i < 8; i++) st(auto, i, i + 1);\n\
              r.push(ld(auto, 7), ld(fx, 3));\n\
              __resize(b, 4);\n\
              for (var i = 0; i < 6; i++) { st(fx, i, 9); st(auto, i, -i); r.push(sh(ld(auto, i)), sh(ld(fx, i))); }\n\
              __resize(b, 16);\n\
              for (var i = 0; i < 17; i += 3) { st(auto, i, i); r.push(sh(ld(auto, i)), sh(ld(fx, i % 5))); }\n\
              var c = __ab(12, 16), w = __ta(5, c, 0), n = __ta(5, c, 4, 2);\n\
              for (var i = 0; i < 4; i++) { st(w, i, i + 100); st(n, i, i + 200); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
              __resize(c, 4);\n\
              for (var i = 0; i < 4; i++) { st(w, i, i + 300); st(n, i, i + 400); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
              __resize(c, 16);\n\
              for (var i = 0; i < 4; i++) { st(w, i, i + 500); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
              print(r.join());",
        spec: Some(
            "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; }\n\
             function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
             var b = __ab(8, 16), auto = __ta(0, b, 0), fx = __ta(0, b, 4, 4), r = [];\n\
             for (var i = 0; i < 8; i++) st(auto, i, i + 1);\n\
             r.push(ld(auto, 7), ld(fx, 3));\n\
             __resize(b, 4);\n\
             for (var i = 0; i < 6; i++) { st(fx, i, 9); st(auto, i, -i); r.push(sh(ld(auto, i)), sh(ld(fx, i))); }\n\
             __resize(b, 16);\n\
             for (var i = 0; i < 17; i += 3) { st(auto, i, i); r.push(sh(ld(auto, i)), sh(ld(fx, i % 5))); }\n\
             var c = __ab(12, 16), w = __ta(5, c, 0), n = __ta(5, c, 4, 2);\n\
             for (var i = 0; i < 4; i++) { st(w, i, i + 100); st(n, i, i + 200); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
             __resize(c, 4);\n\
             for (var i = 0; i < 4; i++) { st(w, i, i + 300); st(n, i, i + 400); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
             __resize(c, 16);\n\
             for (var i = 0; i < 4; i++) { st(w, i, i + 500); r.push(sh(ld(w, i)), sh(ld(n, i))); }\n\
             print(r.join());",
        ),
    },
    Case {
        path: "keyed-view",
        name: "a view as a prototype, a canonical numeric string key, a shared buffer",
        src: "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; }\n\
              function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
              var p0 = __ta(0, __ab(0), 0, 0), o = Object.create(p0), p4 = __ta(5, __ab(16), 0, 4), o4 = Object.create(p4);\n\
              for (var k = 0; k < 3; k++) { st(o, 0, 1); st(p4, 2, 5); }\n\
              Object.prototype[9] = 'x'; Object.prototype['1.5'] = 'p'; Object.prototype['-0'] = 'q';\n\
              var r = [sh(ld(o, 0)), Object.keys(o).length, sh(ld(o4, 9)), sh(ld(o4, 2))];\n\
              st(o4, 2, 7); st(o4, 9, 7);\n\
              r.push(sh(ld(p4, 2)), Object.keys(o4).join('/'), sh(ld(o4, 2)));\n\
              var a = __ta(11, __ab(16), 0, 2);\n\
              for (var k = 0; k < 2; k++) { st(a, '-0', 1); st(a, '1.5', 2); st(a, '01', 3); st(a, 1, 4.5); r.push(sh(ld(a, '-0')), sh(ld(a, '1.5')), ld(a, '01'), ld(a, 1), ld(a, '1'), sh(ld(a, -0))); }\n\
              delete Object.prototype[9]; delete Object.prototype['1.5']; delete Object.prototype['-0'];\n\
              r.push(Object.keys(a).join('/'));\n\
              var s = __sab(8, 32), sf = __ta(3, s, 0, 4), sa = __ta(3, s, 0);\n\
              for (var k = 0; k < 3; k++) for (var i = 0; i < 6; i++) { st(sf, i, i * 1000 - 1); r.push(sh(ld(sa, i))); }\n\
              __grow(s, 16); st(sa, 7, -5); r.push(ld(sa, 7), ld(sf, 3), sh(ld(sf, 4)));\n\
              print(r.join());",
        spec: Some(
            "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; }\n\
             function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
             var p0 = __ta(0, __ab(0), 0, 0), o = Object.create(p0), p4 = __ta(5, __ab(16), 0, 4), o4 = Object.create(p4);\n\
             for (var k = 0; k < 3; k++) { st(o, 0, 1); st(p4, 2, 5); }\n\
             Object.prototype[9] = 'x'; Object.prototype['1.5'] = 'p'; Object.prototype['-0'] = 'q';\n\
             var r = [sh(ld(o, 0)), Object.keys(o).length, sh(ld(o4, 9)), sh(ld(o4, 2))];\n\
             st(o4, 2, 7); st(o4, 9, 7);\n\
             r.push(sh(ld(p4, 2)), Object.keys(o4).join('/'), sh(ld(o4, 2)));\n\
             var a = __ta(11, __ab(16), 0, 2);\n\
             for (var k = 0; k < 2; k++) { st(a, '-0', 1); st(a, '1.5', 2); st(a, '01', 3); st(a, 1, 4.5); r.push(sh(ld(a, '-0')), sh(ld(a, '1.5')), ld(a, '01'), ld(a, 1), ld(a, '1'), sh(ld(a, -0))); }\n\
             delete Object.prototype[9]; delete Object.prototype['1.5']; delete Object.prototype['-0'];\n\
             r.push(Object.keys(a).join('/'));\n\
             var s = __sab(8, 32), sf = __ta(3, s, 0, 4), sa = __ta(3, s, 0);\n\
             for (var k = 0; k < 3; k++) for (var i = 0; i < 6; i++) { st(sf, i, i * 1000 - 1); r.push(sh(ld(sa, i))); }\n\
             __grow(s, 16); st(sa, 7, -5); r.push(ld(sa, 7), ld(sf, 3), sh(ld(sf, 4)));\n\
             print(r.join());",
        ),
    },
];

/// The keyed-view case "each element type" (fast form): every Table 71 type, stores then
/// loads at one site each, three turns (a warm site); BigInts through the BigInt types.
const VIEW_TYPES_FAST: &str = "function ld(a, i) { return a[i]; } function st(a, i, v) { a[i] = v; }\n\
    function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
    var T = [0, 1, 2, 3, 4, 5, 6, 9, 10, 11], V = [1, -1, 300, 70000, -129, 2.5, -2.5, -0, NaN, 1e10, 0.1, 65520, 2147483648, 1 / 3], out = [];\n\
    for (var j = 0; j < T.length; j++) {\n\
      var a = __ta(T[j], __ab(80), 8, 8);\n\
      for (var r = 0; r < 3; r++) for (var i = 0; i < V.length; i++) { st(a, i % 8, V[i]); out.push(sh(ld(a, i % 8))); }\n\
      print(T[j], out.join()); out = [];\n\
    }\n\
    var b = __ta(7, __ab(16), 0, 2), c = __ta(8, __ab(16), 0, 2);\n\
    for (var r = 0; r < 3; r++) { st(b, 0, -1n); st(c, 0, -1n); st(b, 1, 2n ** 63n); st(c, 1, 2n ** 64n + 5n); }\n\
    print(ld(b, 0), ld(c, 0), ld(b, 1), ld(c, 1), ld(b, 2));";

/// The spec form of [`VIEW_TYPES_FAST`].
const VIEW_TYPES_SPEC: &str = "function ld(a, i) { return a[String(i)]; } function st(a, i, v) { a[String(i)] = v; }\n\
    function sh(x) { return Object.is(x, -0) ? '-0' : String(x); }\n\
    var T = [0, 1, 2, 3, 4, 5, 6, 9, 10, 11], V = [1, -1, 300, 70000, -129, 2.5, -2.5, -0, NaN, 1e10, 0.1, 65520, 2147483648, 1 / 3], out = [];\n\
    for (var j = 0; j < T.length; j++) {\n\
      var a = __ta(T[j], __ab(80), 8, 8);\n\
      for (var r = 0; r < 3; r++) for (var i = 0; i < V.length; i++) { st(a, i % 8, V[i]); out.push(sh(ld(a, i % 8))); }\n\
      print(T[j], out.join()); out = [];\n\
    }\n\
    var b = __ta(7, __ab(16), 0, 2), c = __ta(8, __ab(16), 0, 2);\n\
    for (var r = 0; r < 3; r++) { st(b, 0, -1n); st(c, 0, -1n); st(b, 1, 2n ** 63n); st(c, 1, 2n ** 64n + 5n); }\n\
    print(ld(b, 0), ld(c, 0), ld(b, 1), ld(c, 1), ld(b, 2));";

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

// ---- the view makers of the keyed-view cases (TA-2 TA-KEYED; no JS constructor yet) ----

/// The number argument `i` as a byte count, offset or length; `None` when it is undefined.
fn arg_u64(args: &Args<'_>, i: usize) -> Option<u64> {
    let v = args.get(i);
    if v.is_undefined() {
        return None;
    }
    v.as_number().map(|n| n as u64)
}

/// `__ab(bytes, max)`: a new ArrayBuffer (null prototype), resizable to `max` if it is given.
fn host_ab<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let len = arg_u64(&args, 0).unwrap_or(0);
    Ok(cx
        .allocate_array_buffer(Local::null(), len, arg_u64(&args, 1))?
        .value())
}

/// `__sab(bytes, max)`: a new SharedArrayBuffer (null prototype), growable to `max` if given.
fn host_sab<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let len = arg_u64(&args, 0).unwrap_or(0);
    Ok(cx
        .allocate_shared_array_buffer(Local::null(), len, arg_u64(&args, 1))?
        .value())
}

/// `__ta(type, buffer, offset, length)`: a new TypedArray (null prototype) of the Table 71
/// row `type` on `buffer`; no `length`: a length-tracking view (a fixed one on a fixed-length
/// buffer).
fn host_ta<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let t = arg_u64(&args, 0).unwrap_or(0) as u8;
    let off = arg_u64(&args, 2).unwrap_or(0);
    Ok(cx
        .new_typed_array(Local::null(), t, args.get(1), off, arg_u64(&args, 3))?
        .value())
}

/// `__detach(buffer)`: DetachArrayBuffer with the undefined key.
fn host_detach<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.detach_array_buffer(args.get(0), Local::undefined())?;
    Ok(Local::undefined())
}

/// `__resize(buffer, n)`: `ArrayBuffer.prototype.resize`.
fn host_resize<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.array_buffer_resize(args.get(0), arg_u64(&args, 1).unwrap_or(0))?;
    Ok(Local::undefined())
}

/// `__grow(sab, n)`: `SharedArrayBuffer.prototype.grow`.
fn host_grow<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.shared_array_buffer_grow(args.get(0), arg_u64(&args, 1).unwrap_or(0))?;
    Ok(Local::undefined())
}

/// What a run gives: the printed lines and the uncaught exception.
type RunOut = (Vec<String>, Option<String>);

/// The Switch fast path hits of one run, by row name (P0-FASTHITS).
type Hits = Vec<(&'static str, u64)>;

/// Runs the sloppy script `SHOW + src` on a fresh isolate with the human fast paths `on` or
/// off; `stress`: a collection at each allocation, then the heap verifier. On a big stack
/// (debug builds and the overflow cases).
fn run(src: &'static str, on: bool, stress: bool) -> RunOut {
    run_hits(src, on, stress).0
}

/// [`run`], and the Switch fast path hits of the run (the counters are per thread, and the run
/// has its own thread).
fn run_hits(src: &'static str, on: bool, stress: bool) -> (RunOut, Hits) {
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
            for (name, n, f) in [
                ("__ab", 2, host_ab as ep9_vm::HostFn),
                ("__sab", 2, host_sab),
                ("__ta", 4, host_ta),
                ("__detach", 1, host_detach),
                ("__resize", 2, host_resize),
                ("__grow", 2, host_grow),
            ] {
                iso.register_host_function(name, n, f);
            }
            OUT.with(|o| o.borrow_mut().clear());
            let _ = ep9_vm::internals::take_fastdiff_hits();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.set_human_fast_paths(on);
                cx.run_script(unit, "case.js").map(|_| ())
            });
            let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
            let hits = ep9_vm::internals::take_fastdiff_hits();
            iso.collect_garbage();
            iso.verify_heap().expect("a valid heap");
            // The switch is process-wide and keyed by the isolate id: turn it on again, so
            // a dropped isolate leaves no id behind (else every later host call takes the
            // lock; M7-P0 JIT reviewer F5).
            let _ = iso.with_cx(|cx| {
                cx.set_human_fast_paths(true);
                Ok(())
            });
            ((out, r.err().map(|e| e.message)), hits)
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

/// The hits of the row `id` in `h` (0 if it is not a counted row).
fn hits_of(h: &Hits, id: &str) -> u64 {
    h.iter().find(|(n, _)| *n == id).map_or(0, |(_, n)| *n)
}

/// Runs case `c` on and off (and its spec form, and for a Switch row under GC stress with the
/// fast paths on): the mismatches, the number of runs and the hits of its row (on, off).
fn check_case(c: &Case) -> (Vec<String>, usize, (u64, u64)) {
    let started = std::time::Instant::now();
    let p = INVENTORY.iter().find(|p| p.id == c.path).expect("a row");
    let label = format!("{} / {}", c.path, c.name);
    let mut mismatches = Vec::new();
    let (on, hits_on) = run_hits(c.src, true, false);
    let (off, hits_off) = run_hits(c.src, false, false);
    let hits = (hits_of(&hits_on, p.id), hits_of(&hits_off, p.id));
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
    (mismatches, runs, hits)
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
    let mut hits = std::collections::BTreeMap::<&str, (u64, u64)>::new();
    let mut runs = 0;
    for (i, (m, n, (h_on, h_off))) in results {
        *per_path.entry(todo[i].path).or_default() += m.len();
        let h = hits.entry(todo[i].path).or_default();
        (h.0, h.1) = (h.0 + h_on, h.1 + h_off);
        mismatches.extend(m);
        runs += n;
    }
    // P0-FASTHITS: each Switch row whose cases ran took its fast path, and never with the fast
    // paths off.
    for p in INVENTORY.iter().filter(|p| p.off == Off::Switch) {
        let Some(&(h_on, h_off)) = hits.get(p.id) else {
            continue;
        };
        if h_on == 0 {
            mismatches.push(format!("{}: no case took the fast path (0 hits on)", p.id));
            *per_path.entry(p.id).or_default() += 1;
        }
        if h_off != 0 {
            mismatches.push(format!("{}: {h_off} hits with the fast paths off", p.id));
            *per_path.entry(p.id).or_default() += 1;
        }
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
        let h = if p.off == Off::Switch {
            let (h_on, h_off) = hits.get(p.id).copied().unwrap_or((0, 0));
            format!("\"hits_on\":{h_on},\"hits_off\":{h_off},")
        } else {
            String::new()
        };
        let _ = write!(
            j,
            "{}{{\"name\":{},\"site\":{},\"off\":\"{off}\",\"cases\":{n},\"mismatches\":{},{h}\"what\":{}}}",
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
