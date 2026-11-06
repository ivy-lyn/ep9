//! Iterator, Iterator.from and the iterator helpers part 1 (§27.1, M7-P1 SPEC-ITER) at the JS
//! level. Each script prints the lines of node v26.13.0 for the same script (`print` here,
//! `console.log` there), and runs with and without GC stress (`common::run_both`).

mod common;

/// The script prelude: `t(name, f)` prints `name: <result>` or `name: throws <Error name>`;
/// `d(o, k)` is the descriptor kind and attributes of `o[k]`.
macro_rules! script {
    ($body:literal) => {
        concat!(
            "var out = print;\n",
            "function t(name, f) { try { out(name + ': ' + f()); } catch (e) { out(name + ': throws ' + (e && e.constructor && e.constructor.name)); } }\n",
            "function d(o, k) { var x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'none'; return [('value' in x) ? 'data' : 'acc', x.writable, x.enumerable, x.configurable].join(); }\n",
            "var IP = Iterator.prototype;\n",
            "var HP = Object.getPrototypeOf([].values().map(function (x) { return x; }));\n",
            "var WP = Object.getPrototypeOf(Iterator.from({ next: function () {} }));\n",
            $body
        )
    };
}

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The own properties: names, lengths, attributes, key order and prototypes.
#[test]
fn own_properties_match_node() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('typeof', function () { return typeof Iterator + ' ' + Iterator.length + ' ' + Iterator.name; });
t('global desc', function () { return d(globalThis, 'Iterator'); });
t('proto desc', function () { return d(Iterator, 'prototype') + ' ' + (Iterator.prototype === Object.getPrototypeOf(Object.getPrototypeOf([].values()))); });
t('ctor keys', function () { return Object.getOwnPropertyNames(Iterator).filter(function (k) { return k !== 'concat' && k !== 'zip' && k !== 'zipKeyed'; }).join(); });
var mine = ['constructor','reduce','toArray','forEach','some','every','find','map','filter','take','drop','flatMap'];
t('IP keys', function () { return Object.getOwnPropertyNames(IP).filter(function (k) { return mine.indexOf(k) >= 0; }).join(); });
t('IP methods', function () { return mine.slice(1).map(function (k) { return k + ':' + IP[k].length + ':' + IP[k].name + ':' + d(IP, k); }).join(' '); });
t('IP ctor', function () { var x = Object.getOwnPropertyDescriptor(IP, 'constructor'); return d(IP, 'constructor') + ' ' + x.get.name + ' ' + x.set.name + ' ' + x.get.length + ' ' + x.set.length + ' ' + (IP.constructor === Iterator); });
t('HP', function () { return Reflect.ownKeys(HP).map(String).join() + ' ' + HP[Symbol.toStringTag] + ' ' + d(HP, Symbol.toStringTag) + ' ' + HP.next.length + HP.return.length + ' ' + (Object.getPrototypeOf(HP) === IP); });
t('WP', function () { return Reflect.ownKeys(WP).map(String).join() + ' ' + (Object.getPrototypeOf(WP) === IP) + ' ' + d(WP, 'next'); });
"#
        )),
        lines(&[
            "typeof: function 0 Iterator",
            "global desc: data,true,false,true",
            "proto desc: data,false,false,false true",
            "ctor keys: length,name,prototype,from",
            "IP keys: constructor,reduce,toArray,forEach,some,every,find,map,filter,take,drop,flatMap",
            "IP methods: reduce:1:reduce:data,true,false,true toArray:0:toArray:data,true,false,true forEach:1:forEach:data,true,false,true some:1:some:data,true,false,true every:1:every:data,true,false,true find:1:find:data,true,false,true map:1:map:data,true,false,true filter:1:filter:data,true,false,true take:1:take:data,true,false,true drop:1:drop:data,true,false,true flatMap:1:flatMap:data,true,false,true",
            "IP ctor: acc,,false,true get constructor set constructor 0 1 true",
            "HP: next,return,Symbol(Symbol.toStringTag) Iterator Helper data,false,false,true 00 true",
            "WP: next,return true data,true,false,true",
        ])
    );
}

/// Iterator, Iterator.from and the helpers on arrays, strings and generators.
#[test]
fn helpers_match_node() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('call', function () { return Iterator(); });
t('new', function () { return new Iterator(); });
t('sub', function () { class X extends Iterator {} var x = new X(); return (x instanceof Iterator) + ' ' + (Object.getPrototypeOf(Object.getPrototypeOf(x)) === IP); });
t('from string', function () { return Iterator.from('ab').toArray().join(); });
t('from num', function () { return Iterator.from(5); });
t('from iter same', function () { var a = [1].values(); return Iterator.from(a) === a; });
t('from wrap', function () { var n = 0; var w = Iterator.from({ next: function () { return { value: n++, done: n > 3 }; } }); return w.toArray().join() + ' ' + w.return().done + ' ' + Object.keys(w.return()).join(); });
t('wrap next on other', function () { return WP.next.call({}); });
t('helper next on wrap', function () { return HP.next.call(Iterator.from({ next: function () {} })); });
t('map', function () { return [1, 2, 3].values().map(function (x, i) { return x * 10 + i; }).toArray().join(); });
t('filter', function () { return [1, 2, 3, 4].values().filter(function (x, i) { return x % 2 === 0; }).toArray().join(); });
t('take', function () { return [1, 2, 3, 4].values().take(2).toArray().join(); });
t('take inf', function () { return [1, 2, 3].values().take(Infinity).toArray().join(); });
t('take -0.5', function () { return [1, 2].values().take(-0.5).toArray().length; });
t('take neg', function () { return [1].values().take(-1); });
t('take nan', function () { return [1].values().take(); });
t('drop', function () { return [1, 2, 3, 4].values().drop(2).toArray().join(); });
t('drop many', function () { return [1, 2].values().drop(5).toArray().length; });
t('flatMap', function () { return [1, 2].values().flatMap(function (x) { return [x, x * 10]; }).toArray().join(); });
t('flatMap str', function () { return [1].values().flatMap(function (x) { return 'ab'; }).toArray(); });
t('flatMap strobj', function () { return [1].values().flatMap(function (x) { return new String('ab'); }).toArray().join(); });
t('reduce', function () { return [1, 2, 3].values().reduce(function (a, b, i) { return a + b * i; }); });
t('reduce init', function () { return [1, 2, 3].values().reduce(function (a, b, i) { return a + ':' + b + i; }, 'x'); });
t('reduce empty', function () { return [].values().reduce(function () {}); });
t('forEach', function () { var s = []; [5, 6].values().forEach(function (v, i) { s.push(v + '@' + i); }); return s.join(); });
t('some', function () { return [1, 2].values().some(function (v) { return v === 2; }) + ' ' + [1].values().some(function () { return false; }); });
t('every', function () { return [1, 2].values().every(function (v) { return v > 0; }) + ' ' + [1].values().every(function () { return 0; }); });
t('find', function () { return [1, 2, 3].values().find(function (v) { return v > 1; }) + ' ' + [1].values().find(function () {}); });
"#
        )),
        lines(&[
            "call: throws TypeError",
            "new: throws TypeError",
            "sub: true true",
            "from string: a,b",
            "from num: throws TypeError",
            "from iter same: true",
            "from wrap: 0,1,2 true value,done",
            "wrap next on other: throws TypeError",
            "helper next on wrap: throws TypeError",
            "map: 10,21,32",
            "filter: 2,4",
            "take: 1,2",
            "take inf: 1,2,3",
            "take -0.5: 0",
            "take neg: throws RangeError",
            "take nan: throws RangeError",
            "drop: 3,4",
            "drop many: 0",
            "flatMap: 1,10,2,20",
            "flatMap str: throws TypeError",
            "flatMap strobj: a,b",
            "reduce: 9",
            "reduce init: x:10:21:32",
            "reduce empty: throws TypeError",
            "forEach: 5@0,6@1",
            "some: true false",
            "every: true false",
            "find: 2 undefined",
        ])
    );
}

/// Argument checks, IteratorClose, return() in each GeneratorState, a running helper and the setter.
#[test]
fn closing_and_states_match_node() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('non-callable', function () { return [].values().map(1); });
t('non-object this', function () { return IP.map.call(1, function () {}); });
function closable(log) { var i = 0; return { __proto__: IP, next: function () { log.push('next'); return { value: i++, done: false }; }, return: function () { log.push('return'); return {}; } }; }
t('close on bad fn', function () { var log = []; try { closable(log).filter(null); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('take closes', function () { var log = []; closable(log).take(1).toArray(); return log.join(); });
t('some closes', function () { var log = []; closable(log).some(function (v) { return v === 1; }); return log.join(); });
t('return start', function () { var log = []; var h = closable(log).map(function (x) { return x; }); var r = h.return(); return log.join() + ' ' + r.done + ' ' + h.next().done; });
t('return yield', function () { var log = []; var h = closable(log).map(function (x) { return x; }); h.next(); h.return(); return log.join() + ' ' + h.next().done; });
t('return done', function () { var log = []; var h = closable(log).take(0); h.next(); h.return(); return log.join(); });
t('flatMap return', function () { var log = []; var inner = closable(log); var h = [1].values().flatMap(function () { return inner; }); h.next(); h.return(); return log.join(); });
t('running', function () { var h; var it = { __proto__: IP, next: function () { return h.next(); } }; h = it.map(function (x) { return x; }); return h.next(); });
t('running return', function () { var h; var it = { __proto__: IP, next: function () { return h.return(); } }; h = it.map(function (x) { return x; }); return h.next(); });
t('mapper throws', function () { var log = []; try { closable(log).map(function () { throw new RangeError('x'); }).next(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('next not object', function () { var it = { __proto__: IP, next: function () { return 1; }, return: function () { throw 1; } }; return it.map(function (x) { return x; }).next(); });
t('next read once', function () { var n = 0; var it = { __proto__: IP, get next() { n++; var i = 0; return function () { return { done: i++ > 2, value: i }; }; } }; it.map(function (x) { return x; }).toArray(); return n; });
t('tag', function () { return Object.prototype.toString.call([].values().map(function () {})) + ' ' + Object.prototype.toString.call(Iterator.from({ next: function () {} })); });
t('ctor setter', function () { var o = Object.create(IP); o.constructor = 5; return o.constructor + ' ' + d(o, 'constructor') + ' ' + IP.constructor.name; });
t('ctor setter home', function () { 'use strict'; IP.constructor = 1; });
t('gen', function () { function* g() { yield 1; yield 2; yield 3; } return g().map(function (x) { return x * 2; }).filter(function (x) { return x > 2; }).toArray().join(); });
"#
        )),
        lines(&[
            "non-callable: throws TypeError",
            "non-object this: throws TypeError",
            "close on bad fn: return,TypeError",
            "take closes: next,return",
            "some closes: next,next,return",
            "return start: return true true",
            "return yield: next,return true",
            "return done: return",
            "flatMap return: next,return",
            "running: throws TypeError",
            "running return: throws TypeError",
            "mapper throws: next,return,RangeError",
            "next not object: throws TypeError",
            "next read once: 1",
            "tag: [object Iterator Helper] [object Iterator]",
            "ctor setter: 5 data,true,true,true Iterator",
            "ctor setter home: throws TypeError",
            "gen: 4,6",
        ])
    );
}

/// Helper objects keep their iterator, callback, counter and inner iterator in heap slots
/// only: a chain of helpers that allocates in every callback gives the same values with a
/// collection before every allocation (`run_both`). node v26.13.0 prints the same line.
#[test]
fn a_helper_chain_survives_gc_stress() {
    assert_eq!(
        common::run_both(
            r#"
function* g(n) { for (var i = 0; i < n; i++) yield { v: i, s: 'k' + i }; }
var it = g(40)
  .map(function (o, i) { return { v: o.v * 2, s: o.s + ':' + i }; })
  .filter(function (o) { return o.v % 3 !== 0; })
  .flatMap(function (o) { return [o.s, { w: o.v }].values(); })
  .drop(3)
  .take(20);
var parts = [];
var w = Iterator.from({ i: 0, next: function () { return { value: [this.i], done: this.i++ > 4 }; } });
it.forEach(function (x) { parts.push(typeof x === 'string' ? x : x.w); });
print(parts.join(), w.map(function (a) { return a[0]; }).toArray().join());
"#
        ),
        lines(&[
            "4,k4:4,8,k5:5,10,k7:7,14,k8:8,16,k10:10,20,k11:11,22,k13:13,26,k14:14,28,k16:16,32,k17:17 0,1,2,3,4"
        ])
    );
}

/// take and drop: a finite limit above 2^53 - 1 is a RangeError after IteratorClose (the
/// pinned test262, `prototype/take/limit-rangeerror.js`). node v26.13.0 still accepts it.
#[test]
fn a_limit_above_the_safe_integers_is_a_range_error() {
    assert_eq!(
        common::run_both(
            r#"
var closed = 0;
var it = { __proto__: Iterator.prototype, next: function () { return { done: true }; },
           return: function () { closed++; return {}; } };
var r = [];
for (var f of ['take', 'drop']) {
  try { it[f](2 ** 53); r.push('no'); } catch (e) { r.push(e.constructor.name); }
  r.push(it[f](2 ** 53 - 1).next().done, it[f](Infinity).next().done);
}
print(r.join(), closed);
"#
        ),
        lines(&["RangeError,true,true,RangeError,true,true 2"])
    );
}

/// Iterator.from walks the prototype chain of an object that is not an Iterator with the
/// VM's bounded step (Codex r1 SPEC-ITER 1): a proxy that makes the chain a cycle ends in a
/// RangeError, as in node (before, the walk never ended and no interrupt stopped it); a proxy
/// on the way to %Iterator.prototype% still finds it.
#[test]
fn a_proxy_cycle_in_from_is_a_range_error() {
    assert_eq!(
        common::run_both(script!(
            r#"
var a = { [Symbol.iterator]: null, next: function () {} };
var p = new Proxy(a, {});
Object.setPrototypeOf(a, p);
t('from cycle', function () { return Iterator.from(a); });
var b = { next: function () {} };
Object.setPrototypeOf(b, new Proxy(Object.create(IP), {}));
t('from proxy', function () { return Iterator.from(b) === b; });
"#
        )),
        lines(&["from cycle: throws RangeError", "from proxy: true"])
    );
}

/// Part 2 (M7-P2 SPEC-ITER2): Iterator.concat and %Iterator.prototype%[@@dispose]. Each line is
/// node v26.13.0's for the same script, but two: node has no Iterator.zip or zipKeyed
/// ("concat desc"), and node calls the inner iterator's `return` when its `next` throws
/// ("concat throw no close"), where the spec step is `? IteratorStepValue(iteratorRecord)` (no
/// IteratorClose; test262 `concat/next-method-throws.js`; the other helpers do the same).
#[test]
fn concat_and_dispose_match_node() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('concat desc', function () { return Iterator.concat.length + ' ' + Iterator.concat.name + ' ' + d(Iterator, 'concat') + ' ' + Object.getOwnPropertyNames(Iterator).join(); });
t('dispose desc', function () { var f = IP[Symbol.dispose]; return f.length + ' ' + f.name + ' ' + d(IP, Symbol.dispose) + ' ' + Reflect.ownKeys(IP).filter(function (k) { return typeof k === 'symbol'; }).map(String).join(); });
function logged(log, name, vals) { var i = 0; return { [Symbol.iterator]: function () { log.push('open ' + name); return { next: function () { log.push('next ' + name); return i < vals.length ? { value: vals[i++], done: false } : { value: 'x', done: true }; }, return: function () { log.push('return ' + name); return {}; } }; } }; }
t('concat basic', function () { return Iterator.concat([1, 2], ['a', 'b'].values(), new Set([3]), [].values()).toArray().join(); });
t('concat empty', function () { var it = Iterator.concat(); return it.next().done + ' ' + (Object.getPrototypeOf(it) === HP) + ' ' + Object.prototype.toString.call(it); });
t('concat lazy', function () { var log = []; var it = Iterator.concat(logged(log, 'a', [1]), logged(log, 'b', [2])); log.push('made'); var r = []; for (var v of it) r.push(v); return log.join() + ' ' + r.join(); });
t('concat return yield', function () { var log = []; var it = Iterator.concat(logged(log, 'a', [1, 2]), logged(log, 'b', [3])); it.next(); var r = it.return(); return log.join() + ' ' + r.done + ' ' + it.next().done; });
t('concat return start', function () { var log = []; var it = Iterator.concat(logged(log, 'a', [1])); it.return(); return log.join() + '|' + it.next().done; });
t('concat not object', function () { return Iterator.concat([1], 1); });
t('concat no iterator', function () { return Iterator.concat({}); });
t('concat checks first', function () { var log = []; try { Iterator.concat({ get [Symbol.iterator]() { log.push('get'); return function () {}; } }, null); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('concat open not object', function () { var it = Iterator.concat({ [Symbol.iterator]: function () { return 1; } }); return it.next(); });
t('concat fresh result', function () { var res = { value: 1, done: false }; var it = Iterator.concat({ [Symbol.iterator]: function () { return { next: function () { return res; } }; } }); return it.next() !== res; });
t('concat value after done', function () { var n = 0; var src = { [Symbol.iterator]: function () { return { next: function () { return { get value() { n++; }, done: true }; } }; } }; Iterator.concat(src, src).toArray(); return n; });
t('concat throw no close', function () { var log = []; var it = Iterator.concat({ [Symbol.iterator]: function () { return { next: function () { throw new RangeError(); }, return: function () { log.push('return'); } }; } }); try { it.next(); } catch (e) { log.push(e.constructor.name); } return log.join() + ' ' + it.next().done; });
t('concat running', function () { var it; var src = { [Symbol.iterator]: function () { return { next: function () { return it.next(); } }; } }; it = Iterator.concat(src); return it.next(); });
t('concat string obj', function () { return Iterator.concat(Object('ab')).toArray().join(); });
t('dispose', function () { var log = []; var o = Object.create(IP); o.return = function () { log.push('return ' + arguments.length + ' ' + (this === o)); return 5; }; var r = o[Symbol.dispose](); return log.join() + ' ' + r; });
t('dispose none', function () { return IP[Symbol.dispose].call({}) + ' ' + IP[Symbol.dispose].call(1); });
t('dispose bad', function () { return IP[Symbol.dispose].call({ return: 1 }); });
t('dispose undefined', function () { return IP[Symbol.dispose].call(undefined); });
t('dispose helper', function () { var log = []; var h = Iterator.concat(logged(log, 'a', [1, 2])); h.next(); h[Symbol.dispose](); return log.join() + ' ' + h.next().done; });
"#
        )),
        lines(&[
            "concat desc: 0 concat data,true,false,true length,name,prototype,from,concat,zip,zipKeyed",
            "dispose desc: 0 [Symbol.dispose] data,true,false,true Symbol(Symbol.iterator),Symbol(Symbol.toStringTag),Symbol(Symbol.dispose)",
            "concat basic: 1,2,a,b,3",
            "concat empty: true true [object Iterator Helper]",
            "concat lazy: made,open a,next a,next a,open b,next b,next b 1,2",
            "concat return yield: open a,next a,return a true true",
            "concat return start: |true",
            "concat not object: throws TypeError",
            "concat no iterator: throws TypeError",
            "concat checks first: get,TypeError",
            "concat open not object: throws TypeError",
            "concat fresh result: true",
            "concat value after done: 0",
            "concat throw no close: RangeError true",
            "concat running: throws TypeError",
            "concat string obj: a,b",
            "dispose: return 0 true undefined",
            "dispose none: undefined undefined",
            "dispose bad: throws TypeError",
            "dispose undefined: throws TypeError",
            "dispose helper: open a,next a,return a true",
        ])
    );
}

/// Part 2: Iterator.zip and Iterator.zipKeyed (joint iteration). node v26.13.0 has neither: each
/// line is the result of the proposal steps (IteratorZip, IteratorCloseAll: reverse order, the
/// first throw wins), checked by hand against the pinned test262 `built-ins/Iterator/zip*`.
#[test]
fn zip_and_zip_keyed_follow_the_spec() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('zip desc', function () { return [Iterator.zip, Iterator.zipKeyed].map(function (f) { return f.length + ' ' + f.name + ' ' + d(Iterator, f.name); }).join(' | '); });
function src(log, name, n) { var i = 0; return { next: function () { log.push(name + '.next'); i++; return { value: name + i, done: i > n }; }, return: function () { log.push(name + '.return'); return {}; } }; }
function run(it) { var r = []; for (var v of it) r.push(Array.isArray(v) ? '[' + v.join() + ']' : '{' + Reflect.ownKeys(v).map(function (k) { return String(k) + ':' + v[k]; }).join() + '}'); return r.join(' '); }
t('zip shortest', function () { var log = []; var r = run(Iterator.zip([src(log, 'a', 1), src(log, 'b', 3)])); return r + ' ' + log.join(); });
t('zip longest', function () { var log = []; var r = run(Iterator.zip([src(log, 'a', 1), src(log, 'b', 2)], { mode: 'longest', padding: ['p', 'q'] })); return r + ' ' + log.join(); });
t('zip longest nopad', function () { return run(Iterator.zip([[1], [2, 3], []], { mode: 'longest' })); });
t('zip strict ok', function () { return run(Iterator.zip([[1, 2], 'xy'.split('')], { mode: 'strict' })); });
t('zip strict short first', function () { var log = []; var it = Iterator.zip([src(log, 'a', 1), src(log, 'b', 2)], { mode: 'strict' }); it.next(); try { it.next(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zip strict short later', function () { var log = []; var it = Iterator.zip([src(log, 'a', 2), src(log, 'b', 1), src(log, 'c', 2)], { mode: 'strict' }); it.next(); try { it.next(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zip empty', function () { var it = Iterator.zip([]); return it.next().done + ' ' + Object.prototype.toString.call(it); });
t('zip fresh', function () { var it = Iterator.zip([[1, 2]]); var a = it.next().value; var b = it.next().value; return (a !== b) + ' ' + Array.isArray(a); });
t('zip bad mode', function () { return Iterator.zip([], { mode: 'x' }); });
t('zip string mode obj', function () { return Iterator.zip([], { mode: new String('strict') }); });
t('zip bad options', function () { return Iterator.zip([], null); });
t('zip not object', function () { return Iterator.zip('ab'); });
t('zip string item', function () { return Iterator.zip(['ab']); });
t('zip bad padding', function () { return Iterator.zip([], { mode: 'longest', padding: 1 }); });
t('zip pad not read', function () { return Iterator.zip([], { mode: 'shortest', get padding() { throw new RangeError(); } }).next().done; });
t('zip order', function () { var log = []; var o = { get mode() { log.push('mode'); return 'longest'; }, get padding() { log.push('padding'); return { [Symbol.iterator]: function () { log.push('padIter'); return { next: function () { log.push('padNext'); return { done: false, value: 0 }; }, return: function () { log.push('padReturn'); return {}; } }; } }; } }; var items = { [Symbol.iterator]: function () { log.push('items'); var i = 0; return { next: function () { log.push('itemsNext'); return { done: i++ >= 2, value: [] }; } }; } }; Iterator.zip(items, o); return log.join(); });
t('zip close on bad item', function () { var log = []; var items = { [Symbol.iterator]: function () { var a = [src(log, 'a', 1), src(log, 'b', 1), 5]; var i = 0; return { next: function () { return { done: false, value: a[i++] }; }, return: function () { log.push('items.return'); return {}; } }; } }; try { Iterator.zip(items); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zip return start', function () { var log = []; var it = Iterator.zip([src(log, 'a', 1), src(log, 'b', 1)]); it.return(); return log.join() + ' ' + it.next().done; });
t('zip return yield', function () { var log = []; var it = Iterator.zip([src(log, 'a', 3), src(log, 'b', 3), src(log, 'c', 1)], { mode: 'longest' }); it.next(); it.next(); log.length = 0; it.return(); return log.join(); });
t('zip step throws', function () { var log = []; var bad = { next: function () { throw new RangeError(); }, return: function () { log.push('bad.return'); } }; var it = Iterator.zip([src(log, 'a', 2), bad, src(log, 'c', 2)]); try { it.next(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zip return throws', function () { var log = []; var a = { next: function () { return { done: false }; }, return: function () { log.push('a'); throw new RangeError(); } }; var b = { next: function () { return { done: false }; }, return: function () { log.push('b'); throw new SyntaxError(); } }; var it = Iterator.zip([a, b]); it.next(); try { it.return(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zipKeyed basic', function () { var s = Symbol('s'); var o = { a: [1, 2], b: 'xy'.split(''), [s]: [true, false] }; Object.defineProperty(o, 'h', { value: [9], enumerable: false }); o.u = undefined; return run(Iterator.zipKeyed(o)); });
t('zipKeyed proto', function () { var v = Iterator.zipKeyed({ a: [1] }).next().value; return (Object.getPrototypeOf(v) === null) + ' ' + d(v, 'a'); });
t('zipKeyed longest', function () { return run(Iterator.zipKeyed({ a: [1], b: [2, 3] }, { mode: 'longest', padding: { a: 'pa', b: 'pb' } })); });
t('zipKeyed array', function () { return run(Iterator.zipKeyed([[1, 2], [3, 4]])); });
t('zipKeyed not object', function () { return Iterator.zipKeyed(1); });
t('zipKeyed bad value', function () { var log = []; try { Iterator.zipKeyed({ a: src(log, 'a', 1), b: null }); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('zipKeyed pad get throws', function () { var log = []; try { Iterator.zipKeyed({ a: src(log, 'a', 1), b: src(log, 'b', 1) }, { mode: 'longest', padding: { get a() { throw new RangeError(); } } }); } catch (e) { log.push(e.constructor.name); } return log.join(); });
"#
        )),
        lines(&[
            "zip desc: 1 zip data,true,false,true | 1 zipKeyed data,true,false,true",
            "zip shortest: [a1,b1] a.next,b.next,a.next,b.return",
            "zip longest: [a1,b1] [p,b2] a.next,b.next,a.next,b.next,b.next",
            "zip longest nopad: [1,2,] [,3,]",
            "zip strict ok: [1,x] [2,y]",
            "zip strict short first: a.next,b.next,a.next,b.next,b.return,TypeError",
            "zip strict short later: a.next,b.next,c.next,a.next,b.next,c.return,a.return,TypeError",
            "zip empty: true [object Iterator Helper]",
            "zip fresh: true true",
            "zip bad mode: throws TypeError",
            "zip string mode obj: throws TypeError",
            "zip bad options: throws TypeError",
            "zip not object: throws TypeError",
            "zip string item: throws TypeError",
            "zip bad padding: throws TypeError",
            "zip pad not read: true",
            "zip order: mode,padding,items,itemsNext,itemsNext,itemsNext,padIter,padNext,padNext,padReturn",
            "zip close on bad item: b.return,a.return,items.return,TypeError",
            "zip return start: b.return,a.return true",
            "zip return yield: b.return,a.return",
            "zip step throws: a.next,c.return,a.return,RangeError",
            "zip return throws: b,a,SyntaxError",
            "zipKeyed basic: {a:1,b:x,Symbol(s):true} {a:2,b:y,Symbol(s):false}",
            "zipKeyed proto: true data,true,true,true",
            "zipKeyed longest: {a:1,b:2} {a:pa,b:3}",
            "zipKeyed array: {0:1,1:3} {0:2,1:4}",
            "zipKeyed not object: throws TypeError",
            "zipKeyed bad value: a.return,TypeError",
            "zipKeyed pad get throws: b.return,a.return,RangeError",
        ])
    );
}

/// Part 2: chunks and windows (iterator chunking), includes (iterator includes) and join
/// (iterator join). node v26.13.0 has none of them: each line is the result of the proposal
/// steps. The last chunk or partial window is the closure's last Yield: a `return` there
/// closes nothing, and the next `next` calls no `next` of the underlying iterator.
#[test]
fn chunks_windows_includes_and_join_follow_the_spec() {
    assert_eq!(
        common::run_both(script!(
            r#"
t('descs', function () { return ['chunks', 'windows', 'includes', 'join'].map(function (k) { return k + ':' + IP[k].length + ':' + IP[k].name + ':' + d(IP, k); }).join(' '); });
function show(it) { return it.toArray().map(function (a) { return '[' + a.join() + ']'; }).join(''); }
function* g(n) { for (var i = 0; i < n; i++) yield i; }
t('chunks', function () { return show(g(7).chunks(3)) + ' ' + show(g(6).chunks(3)) + ' ' + show(g(0).chunks(3)) + ' ' + show(g(2).chunks(5)); });
t('windows', function () { return show(g(5).windows(3)) + ' ' + show(g(2).windows(3)) + ' ' + show(g(2).windows(3, 'allow-partial')) + ' ' + show(g(4).windows(3, 'allow-partial')) + ' ' + show(g(3).windows(1)); });
t('windows fresh', function () { var it = g(4).windows(2); var a = it.next().value; a[0] = 'x'; var b = it.next().value; return (a !== b) + ' ' + b.join(); });
t('chunks bad size', function () { return [0, -0, -1, 1.5, NaN, Infinity, '2', undefined, 2 ** 32].map(function (s) { try { g(1).chunks(s); return 'ok'; } catch (e) { return e.constructor.name; } }).join(); });
t('chunks max size', function () { return show(g(2).chunks(2 ** 32 - 1)); });
t('windows bad undersized', function () { return [null, '', 'x', true, new String('only-full')].map(function (s) { try { g(1).windows(1, s); return 'ok'; } catch (e) { return e.constructor.name; } }).join(); });
function closable(log, n) { var i = 0; return { __proto__: IP, get next() { log.push('get next'); return function () { log.push('next'); return { value: i++, done: i > n }; }; }, return: function () { log.push('return'); return {}; } }; }
t('chunks close on bad size', function () { var log = []; try { closable(log, 1).chunks(0); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('windows order', function () { var log = []; try { closable(log, 1).windows(0, 'bad'); } catch (e) { log.push(e.constructor.name); } try { closable(log, 1).windows(1, 'bad'); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('chunks return start', function () { var log = []; var c = closable(log, 5).chunks(2); c.return(); return log.join() + ' ' + c.next().done; });
t('chunks return yield', function () { var log = []; var c = closable(log, 5).chunks(2); c.next(); c.return(); return log.join(); });
t('chunks last', function () { var log = []; var c = closable(log, 3).chunks(2); c.next(); var last = c.next(); log.push('|'); var r = c.return(); return log.join() + ' ' + last.value.join() + ' ' + r.done + ' ' + c.next().done; });
t('chunks last next', function () { var log = []; var c = closable(log, 3).chunks(2); c.next(); c.next(); log.push('|'); return c.next().done + ' ' + log.join(); });
t('windows partial last', function () { var log = []; var w = closable(log, 1).windows(3, 'allow-partial'); var a = w.next().value; log.push('|'); w.return(); return a.join() + ' ' + log.join(); });
t('windows return yield', function () { var log = []; var w = closable(log, 5).windows(2); w.next(); w.return(); return log.join(); });
t('chunks running', function () { var c; var it = { __proto__: IP, next: function () { return c.next(); } }; c = it.chunks(1); return c.next(); });
t('includes', function () { return [[1, 2, 3].values().includes(2), [1, 2, 3].values().includes(4), [NaN].values().includes(NaN), [-0].values().includes(0), ['a'].values().includes('a', 1), [4, 5, 6].values().includes(5, 1), [4, 5, 6].values().includes(4, 1), [1].values().includes(1, Infinity)].join(); });
t('includes bad skip', function () { return [NaN, 0.5, -1, -Infinity, '1', null, 2 ** 53, 1n].map(function (s) { try { g(1).includes(0, s); return 'ok'; } catch (e) { return e.constructor.name; } }).join(); });
t('includes close', function () { var log = []; var r = closable(log, 9).includes(2); return r + ' ' + log.join(); });
t('includes no close at end', function () { var log = []; var r = closable(log, 2).includes(7); return r + ' ' + log.join(); });
t('includes close bad skip', function () { var log = []; try { closable(log, 2).includes(0, -1); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('join', function () { return [[1, null, 'b', undefined].values().join(), ['a', 'b'].values().join('-'), [].values().join(), [1, 2].values().join(''), [{ toString: function () { return 'o'; } }].values().join(1)].join(' | '); });
t('join sep order', function () { var log = []; var sep = { toString: function () { log.push('sep'); return '+'; } }; var r = closable(log, 2).join(sep); return r + ' ' + log.join(); });
t('join sep throws', function () { var log = []; try { closable(log, 2).join({ toString: function () { throw new RangeError(); } }); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('join value throws', function () { var log = []; var it = { __proto__: IP, next: function () { log.push('next'); return { value: { toString: function () { throw new RangeError(); } }, done: false }; }, return: function () { log.push('return'); return {}; } }; try { it.join(); } catch (e) { log.push(e.constructor.name); } return log.join(); });
t('join this', function () { return IP.join.call(1); });
t('join symbol', function () { return [Symbol()].values().join(); });
"#
        )),
        lines(&[
            "descs: chunks:1:chunks:data,true,false,true windows:1:windows:data,true,false,true includes:1:includes:data,true,false,true join:1:join:data,true,false,true",
            "chunks: [0,1,2][3,4,5][6] [0,1,2][3,4,5]  [0,1]",
            "windows: [0,1,2][1,2,3][2,3,4]  [0,1] [0,1,2][1,2,3] [0][1][2]",
            "windows fresh: true 1,2",
            "chunks bad size: RangeError,RangeError,RangeError,TypeError,TypeError,TypeError,TypeError,TypeError,RangeError",
            "chunks max size: [0,1]",
            "windows bad undersized: TypeError,TypeError,TypeError,TypeError,TypeError",
            "chunks close on bad size: return,RangeError",
            "windows order: return,RangeError,return,TypeError",
            "chunks return start: get next,return true",
            "chunks return yield: get next,next,next,return",
            "chunks last: get next,next,next,next,next,| 2 true true",
            "chunks last next: true get next,next,next,next,next,|",
            "windows partial last: 0 get next,next,next,|",
            "windows return yield: get next,next,next,return",
            "chunks running: throws TypeError",
            "includes: true,false,true,true,false,true,false,false",
            "includes bad skip: TypeError,TypeError,RangeError,RangeError,TypeError,TypeError,RangeError,TypeError",
            "includes close: true get next,next,next,next,return",
            "includes no close at end: false get next,next,next,next",
            "includes close bad skip: return,RangeError",
            "join: 1,,b, | a-b |  | 12 | o",
            "join sep order: 0+1 sep,get next,next,next,next",
            "join sep throws: return,RangeError",
            "join value throws: next,return,RangeError",
            "join this: throws TypeError",
            "join symbol: throws TypeError",
        ])
    );
}

/// Part 2 helpers keep their lists (the items of concat, the iterators and padding of zip, the
/// ring of windows) in heap slots only: a chain that allocates in every step gives the same
/// values with a collection before every allocation (`run_both`). The values are the spec's
/// (node v26.13.0 has no zip and no windows).
#[test]
fn a_part_2_chain_survives_gc_stress() {
    assert_eq!(
        common::run_both(
            r#"
function* g(n) { for (var i = 0; i < n; i++) yield { v: i, s: 'k' + i }; }
var z = Iterator.zip([g(9), Iterator.concat(g(2), [{ v: 'c', s: 'c' }], g(3)).map(function (o) { return { v: o.v, s: o.s + '!' }; })],
                     { mode: 'longest', padding: [{ v: 'p0' }, { v: 'p1', s: 'pad' }] });
var w = z.windows(3, 'allow-partial').flatMap(function (win) { return win.values(); }).chunks(4);
var k = Iterator.zipKeyed({ a: w, b: [1, 2, 3, 4, 5, 6, 7].values() });
var parts = [];
for (var r of k) parts.push(r.b + ':' + r.a.map(function (p) { return p[0].s + '/' + p[1].s; }).join(' '));
print(parts.join(' | '), [Iterator.concat('ab'.split(''), new Set([1])).join('-'), g(4).map(function (o) { return o.v; }).includes(3, 2)].join());
"#
        ),
        lines(&[
            "1:k0/k0! k1/k1! k2/c! k1/k1! | 2:k2/c! k3/k0! k2/c! k3/k0! | 3:k4/k1! k3/k0! k4/k1! k5/k2! | 4:k4/k1! k5/k2! k6/pad k5/k2! | 5:k6/pad k7/pad k6/pad k7/pad | 6:k8/pad a-b-1,true",
        ])
    );
}

thread_local! {
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `log(...args)`: ToString of each argument, joined by a space.
fn log<'s>(
    cx: &mut ep9_vm::Cx<'s>,
    _this: ep9_vm::Local<'s>,
    args: ep9_vm::Args<'s>,
    _nt: ep9_vm::Local<'s>,
) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    LOG.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(ep9_vm::Local::undefined())
}

fn take_log() -> Vec<String> {
    LOG.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

/// Runs the sloppy script `src` below a 64 KiB pad, so its native frames leave no stale words
/// above the next collection's stack scan (plan rule 35; `weakref.rs` `run`).
#[inline(never)]
fn run_padded(i: &mut ep9_vm::Isolate, src: &str) -> Result<(), String> {
    let mut pad = [0u8; 64 << 10];
    std::hint::black_box(&mut pad);
    let unit = ep9_frontend::compile(
        src,
        ep9_frontend::SourceOpts::script(ep9_frontend::Strictness::Sloppy),
        ep9_frontend::CompileOptions::default(),
    )
    .expect("compiles");
    let r = i
        .with_cx(|cx| cx.run_script(unit, "iterator.js").map(|_| ()))
        .map_err(|e| e.message);
    std::hint::black_box(&mut pad);
    r
}

/// A completed helper drops the values that only its closure used (Codex r1 SPEC-ITER 2), as
/// a completed generator does: each helper below stays reachable from a global, but the
/// object its callback closes over (map run to its end, filter closed by `return` at a yield,
/// a callback that threw, `return` before the first `next`) and the inner iterator of a
/// flatMap closed at a yield die at the next GC. The lines before the GC are node v26.13.0's;
/// the cleanup lines are the spec's (node runs the cleanup jobs in a later task).
#[test]
fn a_completed_helper_drops_its_closure_values() {
    const SRC: &str = r#"
var fr = new FinalizationRegistry(function (held) { log('cleanup', held); });
function mapper(name) { var big = {}; fr.register(big, name); return function (x) { return big ? x : 0; }; }
var m = [1].values().map(mapper('map'));
log(m.next().value, m.next().done);
var f = [1, 2].values().filter(mapper('filter'));
log(f.next().value, f.return().done);
var g = [1].values().flatMap(function (x) { var inner = [x, x].values(); fr.register(inner, 'inner'); return inner; });
log(g.next().value, g.return().done);
var h = [1].values().map((function () { var big = {}; fr.register(big, 'throw'); return function () { if (big) throw new Error('x'); }; })());
try { h.next(); } catch (e) { log(e.message, h.next().done); }
var s = [1].values().map(mapper('start'));
log(s.return().done, s.next().done);
"#;
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = common::isolate(stress);
            i.register_host_function("log", 1, log);
            take_log();
            run_padded(&mut i, SRC).unwrap();
            assert_eq!(
                take_log(),
                ["1 true", "1 true", "1 true", "x true", "true true"],
                "stress={stress}"
            );
            let mut seen = Vec::new();
            common::assert_dies(&mut i, "the closure values of completed helpers", |i| {
                i.with_cx(|cx| cx.cleanup_finalization_registries())
                    .unwrap();
                seen.extend(take_log());
                seen.sort();
                seen == [
                    "cleanup filter",
                    "cleanup inner",
                    "cleanup map",
                    "cleanup start",
                    "cleanup throw",
                ]
            });
            i.verify_heap().expect("a valid heap");
        });
    }
}

/// A completed part 2 helper drops its lists, as a completed generator drops its closure
/// values: the iterable concat has not opened yet (`return` before the first `next`), the
/// padding of a zip run to its end, and the ring of a windows helper run to its end die at
/// the next GC, while each helper stays reachable from a global. The lines before the GC are
/// the spec's (node v26.13.0 has no zip and no windows); the cleanup lines too.
#[test]
fn a_completed_part_2_helper_drops_its_lists() {
    const SRC: &str = r#"
var fr = new FinalizationRegistry(function (held) { log('cleanup', held); });
function reg(name) { var o = { [Symbol.iterator]: function () { return [1].values(); } }; fr.register(o, name); return o; }
var c = Iterator.concat([1], reg('concat'));
log(c.return().done, c.next().done);
var z = Iterator.zip([[1]], { mode: 'longest', padding: [reg('padding')] });
log(z.next().value.length, z.next().done);
var w = [reg('ring')].values().windows(2);
log(w.next().done);
"#;
    for stress in [false, true] {
        common::on_big_stack(move || {
            let mut i = common::isolate(stress);
            i.register_host_function("log", 1, log);
            take_log();
            run_padded(&mut i, SRC).unwrap();
            assert_eq!(
                take_log(),
                ["true true", "1 true", "true"],
                "stress={stress}"
            );
            let mut seen = Vec::new();
            common::assert_dies(&mut i, "the lists of completed part 2 helpers", |i| {
                i.with_cx(|cx| cx.cleanup_finalization_registries())
                    .unwrap();
                seen.extend(take_log());
                seen.sort();
                seen == ["cleanup concat", "cleanup padding", "cleanup ring"]
            });
            i.verify_heap().expect("a valid heap");
        });
    }
}
