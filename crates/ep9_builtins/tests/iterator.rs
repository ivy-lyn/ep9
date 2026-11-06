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
