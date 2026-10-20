var a = { x: 1, y: 2 };
var b = { ...a, y: 3, z: 4 };
print(b.x, b.y, b.z, a.y);
var { x, ...rest } = b;
print(x, rest.y, rest.z, "x" in rest);
var c = { ...null, ...undefined, ...{ q: 1 } };
print(c.q);
