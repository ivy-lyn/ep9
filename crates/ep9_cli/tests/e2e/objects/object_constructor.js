var o = new Object();
o.a = 1;
var p = Object();
print(typeof o, o.a, typeof p, o instanceof Object);
var q = Object(o);
print(q === o);
