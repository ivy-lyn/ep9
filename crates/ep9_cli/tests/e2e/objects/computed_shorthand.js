var k = "dyn", x = 5, y = 6;
var o = { [k + 1]: "v1", [1 + 1]: "two", x, y, m() { return this.x + this.y; } };
print(o.dyn1, o[2], o.x, o.y, o.m());
