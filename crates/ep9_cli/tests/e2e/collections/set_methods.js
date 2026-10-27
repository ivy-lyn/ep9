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
