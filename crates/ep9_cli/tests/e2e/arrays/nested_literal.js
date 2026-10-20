var a = [[1, 2], [3, [4, 5]], { k: [6] }];
print(a[0][1], a[1][1][0], a[2].k[0], a.length, a[1][1].length);
var o = { list: [{ v: 1 }, { v: 2 }], deep: { deeper: { deepest: "yes" } } };
print(o.list[1].v, o.deep.deeper.deepest);
