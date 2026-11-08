//! Explicit resource management at the JS level (M7-P3 SPEC-ERM): `using` and `await using`
//! (BytecodeGen `stmt.rs` "using", runtime ids 48-49), DisposableStack, AsyncDisposableStack,
//! SuppressedError and %AsyncIteratorPrototype%[@@asyncDispose] (`src/disposable.rs`). Each
//! script prints the lines of node v26.13.0 for the same script (`print` here, `console.log`
//! there), and runs with and without GC stress (`common::run_both`).

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// Sync `using`: the end of a block, a function body, a loop body (break, continue), each
/// `for-of` iteration, a `for` head (after the whole loop), a generator `return()`, a class
/// static block, a throw; reverse order, `this` = the value, no argument. (Eval code needs a
/// compiler hook, which the `common` isolates do not have: test262 covers it.)
#[test]
fn using_disposes_at_the_end_of_each_region() {
    assert_eq!(
        common::run_both(
            r#"
var log = [];
function res(n, err) { return { [Symbol.dispose]() { log.push('d' + n + (this.n === n) + arguments.length); if (err) throw err; }, n }; }
function out(s) { print(s + ': ' + log.join()); log = []; }
{ using a = res(1), b = res(2); using c = null, u = undefined; log.push('body'); }
out('block');
function f() { using a = res(1); log.push('ret'); return 7; }
log.push(f()); out('function');
for (let i = 0; i < 3; i++) { using x = res(i); if (i == 1) break; log.push('i' + i); }
out('loop break');
outer: for (const k of [1, 2]) { { using x = res(10 + k); if (k == 1) continue outer; log.push('k' + k); } }
out('continue');
for (using x of [res(5), res(6)]) { log.push('it'); }
out('for-of');
var i = 0;
for (using x = res(9), y = res(8); i < 2; i++) { log.push('loop' + i); }
out('for head');
function* g() { using a = res(3); yield 1; yield 2; }
var it = g(); it.next(); it.return(0); out('generator return');
class C { static { using s = res(4); log.push('static'); } }
out('static block');
try { using a = res(1); throw 'x'; } catch (e) { log.push('caught ' + e); }
out('throw');
var r = res(7); { using z = r; log.push(z === r); }
out('binding');
{ using f1 = { [Symbol.dispose]: function () { log.push('getter ' + typeof this); } }; }
out('this');
"#
        ),
        lines(&[
            "block: body,d2true0,d1true0",
            "function: ret,d1true0,7",
            "loop break: i0,d0true0,d1true0",
            "continue: d11true0,k2,d12true0",
            "for-of: it,d5true0,it,d6true0",
            "for head: loop0,loop1,d8true0,d9true0",
            "generator return: d3true0",
            "static block: static,d4true0",
            "throw: d1true0,caught x",
            "binding: true,d7true0",
            "this: getter object",
        ])
    );
}

/// DisposeResources errors: one error as it is, several as nested SuppressedErrors (the last
/// one disposed outermost), the TypeErrors of CreateDisposableResource, a throwing initializer,
/// one read of @@dispose, TDZ.
#[test]
fn using_errors_follow_dispose_resources() {
    assert_eq!(
        common::run_both(
            r#"
function t(name, f) { try { print(name + ': ' + f()); } catch (e) { print(name + ': throws ' + (e && e.constructor && e.constructor.name) + ' ' + (e instanceof SuppressedError ? [e.error, e.suppressed instanceof SuppressedError ? 'S(' + e.suppressed.error + ',' + e.suppressed.suppressed + ')' : e.suppressed] : e)); } }
function thrower(v) { return { [Symbol.dispose]() { throw v; } }; }
t('one', function () { using a = thrower('E1'); return 'no'; });
t('two', function () { using a = thrower('E1'), b = thrower('E2'); return 'no'; });
t('body and two', function () { using a = thrower('E1'); using b = thrower('E2'); throw 'E3'; });
t('return replaced', function () { using a = thrower('E1'); return 'no'; });
t('not object', function () { using a = 5; return 'no'; });
t('no method', function () { using a = {}; return 'no'; });
t('not callable', function () { using a = { [Symbol.dispose]: 1 }; return 'no'; });
t('init throws first', function () { var log = []; try { using a = { [Symbol.dispose]() { log.push('a'); } }, b = (function () { throw 'I'; })(); } catch (e) { log.push(e); } return log.join(); });
t('getter once', function () { var n = 0; var o = { get [Symbol.dispose]() { n++; return function () {}; } }; { using a = o; } return n; });
t('tdz', function () { { f(); using x = null; function f() { return x; } } });
"#
        ),
        lines(&[
            "one: throws String E1",
            "two: throws SuppressedError E1,E2",
            "body and two: throws SuppressedError E1,S(E2,E3)",
            "return replaced: throws String E1",
            "not object: throws TypeError TypeError: An object is expected with `using` declarations",
            "no method: throws TypeError TypeError: Symbol(Symbol.dispose) is not a function",
            "not callable: throws TypeError TypeError: Symbol(Symbol.dispose) is not a function",
            "init throws first: a,I",
            "getter once: 1",
            "tdz: throws ReferenceError ReferenceError: Cannot access 'x' before initialization",
        ])
    );
}

/// `await using` in async functions, async generators, `for await` and `for` heads: the job
/// order (a ticker counts microtasks) equals node: the step-3.d and step-4 awaits of an
/// undefined or null value, a sync @@dispose fallback whose result is not awaited, errors.
#[test]
fn await_using_awaits_as_node_does() {
    assert_eq!(
        common::run_both(
            r#"
var log = [];
var tick = 0, stop = false;
function ticker() { if (!stop && tick < 300) { tick++; Promise.resolve().then(ticker); } }
function ar(n, err) { return { async [Symbol.asyncDispose]() { log.push('ad' + n + '@' + tick); if (err) throw err; } }; }
function sr(n, err) { return { [Symbol.dispose]() { log.push('sd' + n + '@' + tick); if (err) throw err; } }; }
async function f1() { await using a = ar(1), b = sr(2); using c = sr(3); await using d = null; log.push('body@' + tick); }
async function f2() { await using a = null; log.push('b2@' + tick); }
async function f3() { try { await using a = ar(1, 'E1'), b = sr(2, 'E2'); } catch (e) { log.push(e.error + '/' + e.suppressed + '@' + tick); } }
async function* g() { await using a = ar(7); yield 1; }
async function f4() { for await (await using x of [ar(8), ar(9)]) log.push('it@' + tick); let i = 0; for (await using x = ar(10); i < 2; i++) log.push('L@' + tick); }
async function f5() { await using a = { [Symbol.dispose]() { log.push('sync@' + tick); return { then() { log.push('never'); } }; } }; }
async function main() {
  ticker(); await f1(); log.push('|'); await f2(); log.push('|'); await f3(); log.push('|');
  var it = g(); await it.next(); await it.next(); log.push('|'); await f4(); log.push('|'); await f5(); stop = true;
  print(log.join(' '));
}
main();
"#
        ),
        lines(&[
            "body@1 sd3@2 sd2@2 ad1@3 | b2@5 | sd2@7 ad1@8 E1/E2@9 | ad7@12 | it@16 ad8@16 it@19 ad9@19 L@22 L@22 ad10@22 | sync@24",
        ])
    );
}

/// DisposableStack and SuppressedError: own properties, attributes and key order (node v26),
/// use, adopt, defer, move, dispose, the brand checks and the ReferenceError of a disposed
/// stack.
#[test]
fn disposable_stack_matches_node() {
    assert_eq!(
        common::run_both(
            r#"
function t(name, f) { try { print(name + ': ' + f()); } catch (e) { print(name + ': throws ' + (e && e.constructor && e.constructor.name)); } }
function d(o, k) { var x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'none'; return [('value' in x) ? 'data' : 'acc', x.writable, x.enumerable, x.configurable].join(); }
var DP = DisposableStack.prototype, AP = AsyncDisposableStack.prototype;
t('ctors', function () { return [DisposableStack.length, DisposableStack.name, AsyncDisposableStack.length, SuppressedError.length, SuppressedError.name, d(globalThis, 'DisposableStack'), d(globalThis, 'SuppressedError')].join(' '); });
t('DP keys', function () { return Reflect.ownKeys(DP).map(String).join() + ' ' + (DP[Symbol.dispose] === DP.dispose) + ' ' + d(DP, 'disposed') + ' ' + d(DP, Symbol.toStringTag); });
t('AP keys', function () { return Reflect.ownKeys(AP).map(String).join() + ' ' + (AP[Symbol.asyncDispose] === AP.disposeAsync); });
t('SE proto', function () { return Reflect.ownKeys(SuppressedError.prototype).join() + ' ' + (Object.getPrototypeOf(SuppressedError.prototype) === Error.prototype) + ' ' + SuppressedError.prototype.name; });
t('SE', function () { var e = new SuppressedError(1, 2, 'm'); return [e.error, e.suppressed, e.message, d(e, 'error'), d(e, 'suppressed'), SuppressedError(3, 4).error, 'message' in Object(SuppressedError(3, 4)) ? Object.prototype.hasOwnProperty.call(SuppressedError(3, 4), 'message') : 'x'].join(' '); });
t('no new', function () { return DisposableStack(); });
t('no new async', function () { return AsyncDisposableStack(); });
var log = [];
t('stack', function () { var s = new DisposableStack(); s.use({ [Symbol.dispose]() { log.push('u'); } }); s.use(null); s.defer(function () { log.push('d' + (this === undefined) + arguments.length); }); var r = s.adopt(5, function (v) { log.push('a' + v + arguments.length); }); return [r, s.disposed, s.dispose(), s.disposed, log.join(), s.dispose()].join(' '); });
t('errors', function () { var s = new DisposableStack(); s.defer(function () { throw 1; }); s.defer(function () { throw 2; }); try { s.dispose(); } catch (e) { return [e instanceof SuppressedError, e.error, e.suppressed].join(); } });
t('move', function () { log = []; var s = new DisposableStack(); s.defer(function () { log.push('m'); }); var m = s.move(); var r = [s.disposed, m.disposed, Object.getPrototypeOf(m) === DP]; m.dispose(); return r.join() + ' ' + log.join(); });
t('use after', function () { var s = new DisposableStack(); s.dispose(); s.use({}); });
t('move after', function () { var s = new DisposableStack(); s.dispose(); s.move(); });
t('defer bad', function () { var s = new DisposableStack(); s.defer(1); });
t('use bad', function () { var s = new DisposableStack(); s.use(1); });
t('brand', function () { return DP.dispose.call({}); });
t('brand async', function () { return DP.dispose.call(new AsyncDisposableStack()); });
t('disposed getter brand', function () { return Object.getOwnPropertyDescriptor(DP, 'disposed').get.call({}); });
t('subclass', function () { class S extends DisposableStack {} var s = new S(); return (s instanceof S) + ' ' + Object.prototype.toString.call(s); });
t('using stack', function () { log = []; { using s = new DisposableStack(); s.defer(function () { log.push('x'); }); } return log.join(); });
"#
        ),
        lines(&[
            "ctors: 0 DisposableStack 0 3 SuppressedError data,true,false,true data,true,false,true",
            "DP keys: constructor,use,dispose,adopt,defer,move,disposed,Symbol(Symbol.dispose),Symbol(Symbol.toStringTag) true acc,,false,true data,false,false,true",
            "AP keys: constructor,use,disposeAsync,adopt,defer,move,disposed,Symbol(Symbol.asyncDispose),Symbol(Symbol.toStringTag) true",
            "SE proto: constructor,name,message true SuppressedError",
            "SE: 1 2 m data,true,false,true data,true,false,true 3 false",
            "no new: throws TypeError",
            "no new async: throws TypeError",
            "stack: 5 false  true a51,dfalse0,u ",
            "errors: true,1,2",
            "move: true,false,true m",
            "use after: throws ReferenceError",
            "move after: throws ReferenceError",
            "defer bad: throws TypeError",
            "use bad: throws TypeError",
            "brand: throws TypeError",
            "brand async: throws TypeError",
            "disposed getter brand: throws TypeError",
            "subclass: true [object DisposableStack]",
            "using stack: x",
        ])
    );
}

/// AsyncDisposableStack.prototype.disposeAsync and %AsyncIteratorPrototype%[@@asyncDispose]:
/// the job order (a ticker) equals node. One line differs from node v26.13.0 on purpose: node
/// calls `return` with one argument (`ret 1`); the pinned test262
/// (`built-ins/AsyncIteratorPrototype/Symbol.asyncDispose/invokes-return.js`, step 6.a
/// `Call(return, O, « »)`) asks for none, so ep9 prints `ret 0`.
#[test]
fn async_disposable_stack_matches_node() {
    assert_eq!(
        common::run_both(
            r#"
var log = [];
var tick = 0, stop = false;
function ticker() { if (!stop && tick < 300) { tick++; Promise.resolve().then(ticker); } }
async function main() {
  ticker();
  var s = new AsyncDisposableStack();
  s.use({ async [Symbol.asyncDispose]() { log.push('ua@' + tick); } });
  s.use({ [Symbol.dispose]() { log.push('us@' + tick); return { then() { log.push('never'); } }; } });
  s.use(null);
  s.defer(async function () { log.push('df@' + tick); });
  s.adopt(3, function (v) { log.push('ad' + v + '@' + tick); });
  var p = s.disposeAsync(); log.push('sync@' + tick + ' ' + s.disposed);
  await p; log.push('done@' + tick);
  var e = new AsyncDisposableStack(); e.defer(function () { throw 1; }); e.defer(function () { return Promise.reject(2); });
  try { await e.disposeAsync(); } catch (x) { log.push('err ' + (x instanceof SuppressedError) + ' ' + x.error + ' ' + x.suppressed + '@' + tick); }
  var n = new AsyncDisposableStack(); n.use(undefined); await n.disposeAsync(); log.push('null@' + tick);
  log.push('again ' + (await n.disposeAsync()));
  try { await AsyncDisposableStack.prototype.disposeAsync.call({}); } catch (x) { log.push('brand ' + x.constructor.name); }
  var AIP = Object.getPrototypeOf(Object.getPrototypeOf((async function* () {})()).__proto__);
  var f = AIP[Symbol.asyncDispose]; log.push('aip ' + f.length + ' ' + f.name);
  var r = await f.call({ return(v) { log.push('ret ' + arguments.length + ' ' + v); return 7; } }); log.push('r ' + r);
  r = await f.call({}); log.push('none ' + r);
  try { await f.call({ return() { throw 'T'; } }); } catch (x) { log.push('thrown ' + x); }
  { await using s2 = new AsyncDisposableStack(); s2.defer(function () { log.push('aw@' + tick); }); }
  stop = true;
  print(log.join(' '));
}
main();
"#
        ),
        lines(&[
            "ad3@1 sync@1 true df@2 us@3 ua@4 done@6 err true 1 2@8 null@10 again undefined brand TypeError aip 0 [Symbol.asyncDispose] ret 0 undefined r undefined none undefined thrown T aw@16",
        ])
    );
}
