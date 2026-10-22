// The own properties of Map, Set, WeakMap, WeakSet, their prototypes and the iterator
// prototypes: names, lengths, attributes and order, as node v24 has them.
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
