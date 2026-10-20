var o = { name: "o", hi: function () { return "hi " + this.name; } };
var p = { name: "p", hi: o.hi };
print(o.hi(), p.hi());
var f = o.hi;
var name = "global";
print(f());
