function f(a, b, c, d) { return [a, b, c, d].join("/"); }
var args = [1, 2];
print(f(...args), f(0, ...args, 3), f(...args, ...args), f(...[]));
function Ctor(a, b) { this.s = a + b; }
print(new Ctor(...[3, 4]).s);
