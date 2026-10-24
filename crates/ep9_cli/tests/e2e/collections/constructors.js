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
