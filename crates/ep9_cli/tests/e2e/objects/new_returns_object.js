function F() { this.a = 1; return { b: 2 }; }
function G() { this.a = 1; return 5; }
var f = new F(), g = new G();
print(f.a, f.b, g.a, g instanceof G, f instanceof F);
