class Base { constructor(a, b) { this.sum = a + b; } }
class Derived extends Base {}
var d = new Derived(2, 3);
print(d.sum, d instanceof Base);
class Empty {}
print(typeof new Empty());
