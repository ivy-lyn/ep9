class C { constructor() { this.own = 1; } m() {} }
var ks = [];
for (var k in new C()) ks.push(k);
print(ks.join(","));
