//! Map, Set, WeakMap and WeakSet (§24.1-§24.4) at the JS level (owner M3-P1 B-COLLECTIONS).
//!
//! The expected lines are node v24.22.1's output for the same script (`print` = the
//! arguments' ToString joined by spaces). `common::run_both` runs each script with and
//! without GC stress (a collection before every allocation, then the heap verifier). The
//! scripts are also the e2e programs `crates/ep9_cli/tests/e2e/collections/*.js`.

mod common;

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The own properties of the constructors, prototypes and iterator prototypes (names, lengths, attributes, order).
#[test]
fn own_properties_match_node() {
    assert_eq!(
        common::run_both(
            r#"
function show(o) {
  return Reflect.ownKeys(o).filter(function (k) { return String(k).indexOf('getOrInsert') !== 0; }).map(function (k) {
    var d = Object.getOwnPropertyDescriptor(o, k), f = d.value !== undefined ? d.value : d.get;
    var s = String(k);
    if (typeof f === 'function') s += '(' + f.length + ',' + f.name + ')';
    else if (typeof f === 'string') s += '=' + f;
    return s + (d.get ? '[get]' : '') + (d.enumerable ? 'e' : '') + (d.writable ? 'w' : '') + (d.configurable ? 'c' : '');
  }).join(' ');
}
var names = ['Map', 'Set', 'WeakMap', 'WeakSet'];
for (var i = 0; i < names.length; i++) {
  var C = this[names[i]];
  print(names[i], C.length, C.name, show(C));
  print(show(C.prototype));
  print(Object.getPrototypeOf(C.prototype) === Object.prototype, Object.getPrototypeOf(C) === Function.prototype);
}
var mi = Object.getPrototypeOf(new Map().entries()), si = Object.getPrototypeOf(new Set().values());
var ip = Object.getPrototypeOf(Object.getPrototypeOf([].values()));
print(show(mi), Object.getPrototypeOf(mi) === ip, show(si), Object.getPrototypeOf(si) === ip);
print(Map.prototype[Symbol.iterator] === Map.prototype.entries, Set.prototype.keys === Set.prototype.values, Set.prototype[Symbol.iterator] === Set.prototype.values);
print(Object.prototype.toString.call(new Map()), Object.prototype.toString.call(new WeakSet()), String(new Set().entries()));
"#
        ),
        lines(&[
            "Map 0 Map lengthc name=Mapc prototype groupBy(2,groupBy)wc Symbol(Symbol.species)(0,get [Symbol.species])[get]c",
            "constructor(0,Map)wc get(1,get)wc set(2,set)wc has(1,has)wc delete(1,delete)wc clear(0,clear)wc entries(0,entries)wc forEach(1,forEach)wc keys(0,keys)wc size(0,get size)[get]c values(0,values)wc Symbol(Symbol.toStringTag)=Mapc Symbol(Symbol.iterator)(0,entries)wc",
            "true true",
            "Set 0 Set lengthc name=Setc prototype Symbol(Symbol.species)(0,get [Symbol.species])[get]c",
            "constructor(0,Set)wc has(1,has)wc add(1,add)wc delete(1,delete)wc difference(1,difference)wc clear(0,clear)wc entries(0,entries)wc forEach(1,forEach)wc intersection(1,intersection)wc isSubsetOf(1,isSubsetOf)wc isSupersetOf(1,isSupersetOf)wc isDisjointFrom(1,isDisjointFrom)wc size(0,get size)[get]c symmetricDifference(1,symmetricDifference)wc union(1,union)wc values(0,values)wc keys(0,values)wc Symbol(Symbol.toStringTag)=Setc Symbol(Symbol.iterator)(0,values)wc",
            "true true",
            "WeakMap 0 WeakMap lengthc name=WeakMapc prototype",
            "constructor(0,WeakMap)wc delete(1,delete)wc get(1,get)wc set(2,set)wc has(1,has)wc Symbol(Symbol.toStringTag)=WeakMapc",
            "true true",
            "WeakSet 0 WeakSet lengthc name=WeakSetc prototype",
            "constructor(0,WeakSet)wc delete(1,delete)wc has(1,has)wc add(1,add)wc Symbol(Symbol.toStringTag)=WeakSetc",
            "true true",
            "next(0,next)wc Symbol(Symbol.toStringTag)=Map Iteratorc true next(0,next)wc Symbol(Symbol.toStringTag)=Set Iteratorc true",
            "true true true",
            "[object Map] [object WeakSet] [object Set Iterator]",
        ])
    );
}

/// Insertion order; SameValueZero keys (-0, NaN, numbers, strings by content, objects, symbols); a frozen key.
#[test]
fn keys_are_same_value_zero_in_insertion_order() {
    assert_eq!(
        common::run_both(
            r#"
// Insertion order and SameValueZero keys (§24.1, §24.2).
var m = new Map();
m.set('b', 1).set('a', 2).set(3, 3);
m.set('b', 9);
print([...m.keys()].join(), [...m.values()].join());
m.delete('b'); m.set('b', 4);
print([...m.entries()].map(function (e) { return e[0] + '=' + e[1]; }).join(' '));
m.set(-0, 'zero');
print(m.has(0), m.get(+0), 1 / [...m.keys()][3], m.size);
m.set(NaN, 'nan');
print(m.get(0 / 0), m.get(Number('x')), m.has(NaN));
m.set(1, 'num'); m.set('1', 'str');
print(m.get(1), m.get('1'), m.get(1.0), m.size);
var s1 = 'ab' + 'cd', s2 = ['a', 'bcd'].join('');
m.set(s1, 'content');
print(m.get(s2), m.get('abcd'));
var o1 = {}, o2 = {};
m.set(o1, 'o1');
print(m.get(o1), m.get(o2), m.has({}));
var sym = Symbol('k');
m.set(sym, 'sym');
print(m.get(sym), m.get(Symbol('k')));
m.set(undefined, 'u').set(null, 'n').set(true, 't');
print(m.get(undefined), m.get(null), m.get(true), m.get(false), m.get());
var s = new Set([2, 1, 2, -0, 0, NaN, NaN, '2']);
print(s.size, [...s].map(String).join(), 1 / [...s][2]);
var f = Object.freeze({ frozen: true });
var fm = new Map([[f, 1]]);
print(fm.get(f), Object.isFrozen(f), Object.keys(f).join(), Object.getOwnPropertyNames(f).length, Object.getOwnPropertySymbols(f).length);
"#
        ),
        lines(&[
            "b,a,3 9,2,3",
            "a=2 3=3 b=4",
            "true zero Infinity 4",
            "nan nan true",
            "num str num 7",
            "content content",
            "o1 undefined false",
            "sym undefined",
            "u n t undefined u",
            "5 2,1,0,NaN,2 Infinity",
            "1 true frozen 1 0",
        ])
    );
}

/// forEach and iterators across deletes, adds, a rehash and a clear; done stays done; the iterator brand.
#[test]
fn iteration_sees_additions_and_skips_deletions() {
    assert_eq!(
        common::run_both(
            r#"
// Iteration sees entries added during the walk and skips deleted ones; a done iterator
// stays done; a rehash or clear does not lose the place.
var s = new Set([1, 2, 3]);
var seen = [];
s.forEach(function (v) {
  seen.push(v);
  if (v === 1) { s.delete(2); s.add(4); }
  if (v === 4) { s.delete(1); s.add(1); }
});
print(seen.join());
var m = new Map([['a', 1], ['b', 2]]);
var it = m.keys();
print(it.next().value);
for (var i = 0; i < 100; i++) m.set('k' + i, i);
m.delete('b');
var rest = [];
for (var k of it) rest.push(k);
print(rest.length, rest[0], rest[99]);
var it2 = m.values();
m.clear();
m.set('new', 7);
print(JSON_like(it2.next()), JSON_like(it2.next()));
function JSON_like(r) { return r.value + '/' + r.done; }
var done = new Set([1]).values();
done.next(); done.next();
print(JSON_like(done.next()));
var e = new Map([[1, 'x']]).entries().next().value;
print(Array.isArray(e), e.length, e[0], e[1]);
var se = new Set(['v']).entries().next().value;
print(se[0], se[1]);
var it3 = new Map()[Symbol.iterator]();
print(it3[Symbol.iterator]() === it3, Object.prototype.toString.call(it3));
try { it3.next.call({}); } catch (err) { print(err.constructor.name); }
try { it3.next.call(new Set().values()); } catch (err) { print(err.constructor.name); }
var thisArgs = [];
new Map([[1, 2]]).forEach(function (v, k, map) { thisArgs.push(this.tag, v, k, map instanceof Map); }, { tag: 'T' });
print(thisArgs.join());
try { new Set().forEach(1); } catch (err) { print(err.constructor.name); }
var big = new Set();
for (var j = 0; j < 5000; j++) big.add(j);
for (var j = 0; j < 5000; j += 3) big.delete(j);
var sum = 0, count = 0;
big.forEach(function (v) { sum += v; count++; });
print(big.size, count, sum);
"#
        ),
        lines(&[
            "1,3,4,1",
            "a",
            "100 k0 k99",
            "7/false undefined/true",
            "undefined/true",
            "true 2 1 x",
            "v v",
            "true [object Map Iterator]",
            "TypeError",
            "TypeError",
            "T,2,1,true",
            "TypeError",
            "3333 3333 8331667",
        ])
    );
}

/// The seven Set methods: results, GetSetRecord errors, the call order on a set-like, -0, a growing receiver.
#[test]
fn set_methods_follow_get_set_record() {
    assert_eq!(
        common::run_both(
            r#"
// Set methods (§24.2.4): results in order, GetSetRecord, the call order on a set-like.
function show(s) { return [...s].join() + '|' + s.size; }
var a = new Set([1, 2, 3, 4]), b = new Set([3, 4, 5]);
print(show(a.union(b)), show(a.intersection(b)), show(a.difference(b)), show(a.symmetricDifference(b)));
print(a.isSubsetOf(b), new Set([3]).isSubsetOf(b), a.isSupersetOf(new Set([1, 4])), a.isDisjointFrom(new Set([9])), a.isDisjointFrom(b));
print(show(new Set([1, 2]).union(new Map([[2, 'x'], [7, 'y']]))));
var log = [];
var like = {
  get size() { log.push('size'); return 2; },
  get has() { log.push('has'); return function (v) { log.push('has(' + v + ')'); return v === 2; }; },
  get keys() {
    log.push('keys');
    return function () {
      var vals = [2, 9], i = 0;
      return { next: function () { log.push('next'); return i < vals.length ? { value: vals[i++], done: false } : { done: true }; },
               return: function () { log.push('return'); return {}; } };
    };
  }
};
print(show(new Set([1, 2, 3]).intersection(like)), log.join());
log = [];
print(show(new Set([1]).intersection(like)), log.join());
log = [];
print(new Set([1, 2, 3]).isSupersetOf(like), log.join());
log = [];
print(new Set([2, 5, 6]).isDisjointFrom(like), log.join());
log = [];
print(show(new Set([1, 2, 3]).difference(like)), log.join());
log = [];
print(show(new Set([1, 2]).symmetricDifference(like)), log.join());
var errs = [];
[undefined, 1, {}, { size: -1, has: function () {}, keys: function () {} }, { size: 1, has: 1, keys: function () {} },
 { size: 1, has: function () {}, keys: 'k' }, { size: 'x', has: function () {}, keys: function () {} }].forEach(function (o) {
  try { new Set().union(o); errs.push('ok'); } catch (e) { errs.push(e.constructor.name); }
});
print(errs.join());
try { Set.prototype.union.call(new Map(), new Set()); } catch (e) { print(e.constructor.name); }
var z = new Set([0]).union({ size: 1, has: function () {}, keys: function () { var d = false; return { next: function () { var r = d ? { done: true } : { value: -0, done: false }; d = true; return r; } }; } });
print(z.size, 1 / [...z][0]);
var inf = new Set([1, 2]).isSubsetOf({ size: Infinity, has: function () { return true; }, keys: function () {} });
print(inf);
class MySet extends Set {}
var r = new MySet([1]).union(new Set([2]));
print(r instanceof MySet, Object.getPrototypeOf(r) === Set.prototype);
var grow = new Set([1, 2]);
var visits = [];
grow.isSubsetOf({ size: 10, has: function (v) { visits.push(v); if (v === 1) grow.add(3); return true; }, keys: function () {} });
print(visits.join());
"#
        ),
        lines(&[
            "1,2,3,4,5|5 3,4|2 1,2|2 1,2,5|3",
            "false true true true false",
            "1,2,7|3",
            "2|1 size,has,keys,next,next,next",
            "|0 size,has,keys,has(1)",
            "false size,has,keys,next,next,return",
            "false size,has,keys,next,return",
            "1,3|2 size,has,keys,next,next,next",
            "1,9|2 size,has,keys,next,next,next",
            "TypeError,TypeError,TypeError,RangeError,TypeError,TypeError,TypeError",
            "TypeError",
            "1 Infinity",
            "true",
            "false true",
            "1,2,3",
        ])
    );
}

/// The constructors: iterables, the adder, iterator close, new.target and @@species, receiver checks.
#[test]
fn constructors_add_from_iterables() {
    assert_eq!(
        common::run_both(
            r#"
// Constructors: iterables, the adder, iterator close, new.target, @@species.
var m = new Map([[1, 'a'], [2, 'b']]);
print(m.size, new Map().size, new Map(undefined).size, new Map(null).size);
var s = new Set('hello');
print([...s].join(''), new Set([]).size);
try { Map(); } catch (e) { print(e.constructor.name); }
try { Set([]); } catch (e) { print(e.constructor.name); }
try { new Map([1]); } catch (e) { print(e.constructor.name); }
try { new Map(5); } catch (e) { print(e.constructor.name); }
var closed = 0;
var iterable = {};
iterable[Symbol.iterator] = function () {
  var n = 0;
  return { next: function () { n++; return n < 3 ? { value: n === 2 ? 'bad' : [n, n], done: false } : { done: true }; },
           return: function () { closed++; return {}; } };
};
try { new Map(iterable); } catch (e) { print(e.constructor.name, closed); }
var calls = [];
class LoggingMap extends Map {
  set(k, v) { calls.push(k + ':' + v); return super.set(k, v); }
}
var lm = new LoggingMap([['x', 1], ['y', 2]]);
print(calls.join(), lm.get('y'), lm instanceof LoggingMap, lm instanceof Map);
class LoggingSet extends Set {
  add(v) { calls.push('add ' + v); return super.add(v); }
}
calls = [];
new LoggingSet([5, 6]);
print(calls.join());
var oldAdd = Set.prototype.add;
Set.prototype.add = 1;
try { new Set([1]); } catch (e) { print(e.constructor.name); }
print(new Set().size);
Set.prototype.add = oldAdd;
print(Map[Symbol.species] === Map, Set[Symbol.species] === Set);
function F() {}
F.prototype = Map.prototype;
var viaReflect = Reflect.construct(Map, [], F);
print(viaReflect instanceof Map, Object.getPrototypeOf(viaReflect) === Map.prototype);
var odd = Reflect.construct(Set, [[1]], function () {}.bind());
print(Object.getPrototypeOf(odd) === Set.prototype);
try { Map.prototype.get.call({}, 1); } catch (e) { print(e.constructor.name); }
try { Map.prototype.get.call(new Set(), 1); } catch (e) { print(e.constructor.name); }
try { Set.prototype.has.call(new Map(), 1); } catch (e) { print(e.constructor.name); }
try { Object.getOwnPropertyDescriptor(Map.prototype, 'size').get.call(new WeakMap()); } catch (e) { print(e.constructor.name); }
print(new Map().set(1, 1) instanceof Map, new Set().add(1).has(1), new Map([[1, 1]]).delete(1), new Set().delete(1), new Map().clear());
"#
        ),
        lines(&[
            "2 0 0 0",
            "helo 0",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError 1",
            "x:1,y:2 2 true true",
            "add 5,add 6",
            "TypeError",
            "0",
            "true true",
            "true true",
            "true",
            "TypeError",
            "TypeError",
            "TypeError",
            "TypeError",
            "true true true false undefined",
        ])
    );
}

/// WeakMap and WeakSet: CanBeHeldWeakly keys, the methods, many keys across a GC.
#[test]
fn weak_collections_take_objects_and_unregistered_symbols() {
    assert_eq!(
        common::run_both(
            r#"
// WeakMap and WeakSet: CanBeHeldWeakly keys (§9.13), the methods, GC.
var wm = new WeakMap();
var k1 = {}, k2 = function () {}, k3 = Symbol('local');
wm.set(k1, 'obj').set(k2, 'fn').set(k3, 'sym');
print(wm.get(k1), wm.get(k2), wm.get(k3), wm.has({}), wm.get(1), wm.has('x'), wm.delete(2));
var bad = [1, 'str', null, undefined, true, Symbol.for('registered')];
var out = [];
bad.forEach(function (k) { try { wm.set(k, 1); out.push('ok'); } catch (e) { out.push(e.constructor.name); } });
print(out.join());
print(wm.delete(k1), wm.has(k1), wm.delete(k1), wm.get(k1));
var ws = new WeakSet([k1, k2]);
print(ws.has(k1), ws.has(k3), ws.add(k3) === ws, ws.has(k3), ws.delete(k2), ws.has(k2));
out = [];
bad.forEach(function (k) { try { ws.add(k); out.push('ok'); } catch (e) { out.push(e.constructor.name); } });
print(out.join());
print(new WeakMap([[k1, 5]]).get(k1), Symbol.iterator in WeakMap.prototype, 'size' in WeakSet.prototype, 'clear' in WeakMap.prototype);
try { WeakMap.prototype.set.call(new Map(), {}, 1); } catch (e) { print(e.constructor.name); }
try { WeakSet.prototype.add.call(new Set(), {}); } catch (e) { print(e.constructor.name); }
try { new WeakMap([[1, 2]]); } catch (e) { print(e.constructor.name); }
var keep = [];
for (var i = 0; i < 2000; i++) {
  var k = { i: i };
  wm.set(k, { payload: i });
  if (i % 100 === 0) keep.push(k);
}
var total = 0;
keep.forEach(function (k) { total += wm.get(k).payload; });
print(keep.length, total, wm.has(keep[3]));
// A key the GC cleared is not undefined: get() and has(undefined) find nothing.
print(wm.get(), wm.has(undefined), wm.delete(undefined));
var wk = Symbol.iterator;
wm.set(wk, 'well-known');
print(wm.get(wk));
"#
        ),
        lines(&[
            "obj fn sym false undefined false false",
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "true false false undefined",
            "true false true true true false",
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "5 false false false",
            "TypeError",
            "TypeError",
            "TypeError",
            "20 19000 true",
            "undefined false false",
            "well-known",
        ])
    );
}

/// Map.groupBy: SameValueZero groups in first-seen order, errors, iterator close.
#[test]
fn map_group_by() {
    assert_eq!(
        common::run_both(
            r#"
// Map.groupBy (§24.1.2.1): SameValueZero groups in first-seen order.
var g = Map.groupBy([1, 2, 3, 4, 5, 6], function (v, i) { return v % 3; });
print([...g.keys()].join(), g.get(0).join(), g.get(1).join(), Array.isArray(g.get(2)));
var z = Map.groupBy([-0, 0, NaN, 0 / 0], function (v) { return v; });
print(z.size, 1 / [...z.keys()][0], z.get(NaN).length);
var o = {};
var idx = [];
var go = Map.groupBy('abc', function (c, i) { idx.push(i); return c === 'b' ? o : 'other'; });
print(go.get(o).join(), go.get('other').join(), idx.join(), go instanceof Map);
try { Map.groupBy(null, function () {}); } catch (e) { print(e.constructor.name); }
try { Map.groupBy([], 1); } catch (e) { print(e.constructor.name); }
var closed = 0;
var it = {};
it[Symbol.iterator] = function () { return { next: function () { return { value: 1, done: false }; }, return: function () { closed++; return {}; } }; };
try { Map.groupBy(it, function () { throw new RangeError('stop'); }); } catch (e) { print(e.constructor.name, closed); }
"#
        ),
        lines(&[
            "1,2,0 3,6 1,4 true",
            "2 Infinity 2",
            "b a,c 0,1,2 true",
            "TypeError",
            "TypeError",
            "RangeError 1",
        ])
    );
}
