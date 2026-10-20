var a = null, b = 0, c = "";
print(a ?? "d", b ?? "d", c ?? "d", undefined ?? null ?? 3);
var o = { x: null, y: 1, z: 0 };
o.x ??= "set"; o.y ??= "no"; o.z ||= 5; o.y &&= 7;
print(o.x, o.y, o.z);
