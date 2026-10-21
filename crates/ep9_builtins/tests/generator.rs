//! JS-level generator tests (P1-GEN-VM, M2-P1-GEN-JSTESTS). They need BytecodeGen support
//! (P1-GEN-CG), which merges after P1-GEN-VM (plan §9), so they are ignored on the GEN-VM
//! branch; the P1 integration fixer removes the ignores after the GEN-CG merge. The expected
//! lines come from node v24 (`print` = `console.log`). Every check runs with and without GC
//! stress (`common::run_both`), except the deep chain (GC stress is too slow for it).

mod common;

use common::{on_big_stack, run, run_both};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// next/return/throw, sent values, the completed state.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn basic() {
    assert_eq!(
        run_both(
            r#"function* g(a) { const x = yield a; yield x + 1; return a * 10; }
var it = g(5), r;
r = it.next(99); print(r.value, r.done);
r = it.next(7); print(r.value, r.done);
r = it.next(); print(r.value, r.done);
r = it.next(); print(r.value, r.done);
r = it.return(3); print(r.value, r.done);
try { it.throw(9); } catch (e) { print('caught', e); }"#
        ),
        lines(&[
            "5 false",
            "8 false",
            "50 true",
            "undefined true",
            "3 true",
            "caught 9"
        ])
    );
}

/// return and throw run `finally` blocks; a yield inside `finally`; a caught throw.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn finally() {
    assert_eq!(
        run_both(
            r#"function* g() { try { yield 1; yield 2; } finally { print('fin'); } }
var it = g(); print(it.next().value);
var r = it.return(5); print(r.value, r.done);
var it2 = g(); it2.next();
try { it2.throw(new Error('boom')); } catch (e) { print(e.message); }
function* h() { try { yield 1; } finally { yield 'f'; } }
var it3 = h(); it3.next(); r = it3.return(8); print(r.value, r.done); r = it3.next(); print(r.value, r.done);
function* k() { try { yield 1; } catch (e) { print('in', e); yield e * 2; } return 'end'; }
var it4 = k(); it4.next(); r = it4.throw(21); print(r.value, r.done); r = it4.next(); print(r.value, r.done);
var it5 = g(); r = it5.return(4); print(r.value, r.done);"#
        ),
        lines(&[
            "1", "fin", "5 true", "fin", "boom", "f false", "8 true", "in 21", "42 false",
            "end true", "4 true"
        ])
    );
}

/// `yield*`: values, sent values, return and throw forwarding, the missing `throw` method (§15.5.5 step 7.b.iii), strings and arrays.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn delegate() {
    assert_eq!(
        run_both(
            r#"function* inner() { var x = yield 1; print('inner got', x); try { yield 2; } finally { print('inner fin'); } return 'r'; }
function* outer() { var v = yield* inner(); print('outer got', v); yield 3; }
var it = outer(), r;
r = it.next(); print(r.value, r.done);
r = it.next('a'); print(r.value, r.done);
r = it.return('z'); print(r.value, r.done);
var it2 = outer(); it2.next(); it2.next();
try { it2.throw('t'); } catch (e) { print('thrown', e); }
var o = { [Symbol.iterator]() { return { next() { return { value: 1, done: false }; }, return() { print('closed'); return {}; } }; } };
function* d() { yield* o; }
var id = d(); id.next();
try { id.throw(5); } catch (e) { print(e.constructor.name, e.message); }
function* s() { yield* 'ab'; yield* [3, 4]; }
print([...s()].join());"#
        ),
        lines(&[
            "1 false",
            "inner got a",
            "2 false",
            "inner fin",
            "z true",
            "inner got undefined",
            "inner fin",
            "thrown t",
            "closed",
            "TypeError The iterator does not provide a 'throw' method.",
            "a,b,3,4"
        ])
    );
}

/// yields inside loops (a SwitchOnSmi at each loop header), labeled continue, for-of, closures over loop bindings, early break closes the generator.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn loops() {
    assert_eq!(
        run_both(
            r#"function* range(n) { for (let i = 0; i < n; i++) yield i; }
print([...range(5)].join());
function* nested() { outer: for (var i = 0; i < 3; i++) { for (var j = 0; j < 3; j++) { if (j === 2) continue outer; yield i * 10 + j; } } }
print([...nested()].join());
function* w() { var i = 0; while (true) { var cmd = yield i++; if (cmd === 'stop') return 'done'; } }
var it = w(); it.next(); it.next(); print(it.next().value, it.next('stop').value);
function* fo() { for (var x of [1, 2, 3]) { yield x * 2; } }
var sum = 0; for (var v of fo()) sum += v; print(sum);
function* closures() { let fs = []; for (let i = 0; i < 3; i++) { fs.push(() => i); yield i; } let s = ''; for (const f of fs) s += f(); yield s; }
print([...closures()].join('|'));
function* early() { try { yield 1; yield 2; } finally { print('cleanup'); } }
for (var e of early()) { print('got', e); break; }
var [a, b] = range(10); print(a, b);"#
        ),
        lines(&[
            "0,1,2,3,4",
            "0,1,10,11,20,21",
            "2 done",
            "12",
            "0|1|2|012",
            "got 1",
            "cleanup",
            "0 1"
        ])
    );
}

/// the prototype chain of generator functions and generator objects (§15.5.3, §15.5.4, §27.3.3, §27.5.1), generator methods.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn objects() {
    assert_eq!(
        run_both(
            r#"// inChain(P, x): P is on x's prototype chain (no Object.getPrototypeOf in ep9 yet).
function inChain(P, x) { function F() {} F.prototype = P; return x instanceof F; }
function* g() { yield 1; }
var GF = g.constructor;
print(GF.name, GF === (function* () {}).constructor, GF.prototype !== Function.prototype);
print(inChain(GF.prototype, g), inChain(Function.prototype, GF.prototype), g instanceof Function);
print(inChain(GF.prototype.prototype, g.prototype), g() instanceof g, inChain(GF.prototype.prototype, g()));
print(g.prototype.hasOwnProperty('constructor'), Object.getOwnPropertyNames(g.prototype).length);
var d = Object.getOwnPropertyDescriptor(g, 'prototype'); print(d.writable, d.enumerable, d.configurable);
print(typeof GF.prototype.prototype.next, typeof GF.prototype.prototype.return, typeof GF.prototype.prototype.throw);
print(Object.prototype.toString.call(g()), Object.prototype.toString.call(g));
var it = g(); print(it[Symbol.iterator]() === it);
try { new g(); } catch (e) { print(e.constructor.name); }
g.prototype = 1; var it2 = g(); print(inChain(GF.prototype.prototype, it2), typeof it2.next);
var o = { *m() { yield 'm'; } }; class C { *m() { yield 'c'; } static *s() { yield 's'; } }
print(o.m().next().value, new C().m().next().value, C.s().next().value);
print(o.m.constructor === GF, o.m.hasOwnProperty('prototype'));"#
        ),
        lines(&[
            "GeneratorFunction true true",
            "true true true",
            "true true true",
            "false 0",
            "true false false",
            "function function function",
            "[object Generator] [object GeneratorFunction]",
            "true",
            "TypeError",
            "true function",
            "m c s",
            "true true"
        ])
    );
}

/// `this` and the arguments object survive suspends; parameter defaults and rest.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn this_args() {
    assert_eq!(
        run_both(
            r#"function* g() { yield arguments.length; yield this.x; yield arguments[2]; }
var o = { x: 'ox', g: g }; var it = o.g(1, 2, 3);
print(it.next().value, it.next().value, it.next().value);
function* p(a, b = a + 1, ...rest) { yield a; yield b; yield rest.length; }
print([...p(1)].join(), [...p(1, 5, 7, 8)].join());
'use strict';
function* s() { 'use strict'; yield this; } print(s().next().value);"#
        ),
        lines(&["3 ox 3", "1,2,0 1,5,2", "undefined"])
    );
}

/// the executing-state TypeError; a throw from the body closes the generator; GeneratorValidate.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn states() {
    assert_eq!(
        run_both(
            r#"var it; function* g() { it.next(); }
it = g();
try { it.next(); } catch (e) { print(e.constructor.name, e.message); }
print(it.next().done);
function* h() { yield 1; throw new Error('body'); }
var j = h(); j.next();
try { j.next(); } catch (e) { print(e.message); }
print(j.next().done);
var next = j.next;
try { next.call({}); } catch (e) { print(e.constructor.name); }
try { next.call(undefined); } catch (e) { print(e.constructor.name); }"#
        ),
        lines(&[
            "TypeError Generator is already running",
            "true",
            "body",
            "true",
            "TypeError",
            "TypeError"
        ])
    );
}

/// A 10^5-deep `yield*` chain: each level resumes the next one from a native `next`, one
/// checked native level each (`call::run_resumed`), so the overflow is a catchable RangeError
/// (plan-v3 P1-GEN-VM acceptance). No GC stress.
#[test]
#[ignore = "needs P1-GEN-CG"]
fn a_deep_yield_star_chain_is_a_catchable_range_error() {
    let src = r#"function* d(n) { if (n > 0) yield* d(n - 1); else yield 0; }
try { d(1e5).next(); print('no error'); } catch (e) { print(e instanceof RangeError); }
print(d(100).next().value);"#;
    assert_eq!(on_big_stack(move || run(src, false)), lines(&["true", "0"]));
}

/// M2-P1-GENFN-PROTO: %GeneratorFunction%'s [[Prototype]] is %Function% (§27.3.2). The P1
/// integration fixer sets it in `builtins::install`; until then this prints "false true".
#[test]
#[ignore = "needs P1-FIX %Function% proto"]
fn generator_function_constructor_inherits_from_function() {
    assert_eq!(
        run_both(
            r#"// %GeneratorFunction%'s [[Prototype]] is %Function% (no Object.getPrototypeOf in ep9 yet).
function inChain(P, x) { function F() {} F.prototype = P; return x instanceof F; }
var GF = (function* () {}).constructor;
print(inChain(Function, GF), GF instanceof Function);"#
        ),
        lines(&["true true"])
    );
}
