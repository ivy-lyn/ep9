function f() {}
var o = { m() {}, a: () => 1 };
print(typeof f, typeof o.m, typeof o.a, typeof f(), typeof new f(), typeof f.prototype);
print(typeof typeof 1, typeof (1 + "x"), typeof (1 - "x"), typeof !1);
