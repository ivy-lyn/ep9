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
gc();
var total = 0;
keep.forEach(function (k) { total += wm.get(k).payload; });
print(keep.length, total, wm.has(keep[3]));
// A key the GC cleared is not undefined: get() and has(undefined) find nothing.
print(wm.get(), wm.has(undefined), wm.delete(undefined));
var wk = Symbol.iterator;
wm.set(wk, 'well-known');
print(wm.get(wk));
