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
