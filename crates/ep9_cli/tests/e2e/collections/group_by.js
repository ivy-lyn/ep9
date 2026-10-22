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
