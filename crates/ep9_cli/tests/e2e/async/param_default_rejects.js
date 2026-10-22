// An error in a parameter default rejects the promise; the call itself does not throw.
async function f(a = missing) { print("never"); }
var p = f();
print("called");
p.catch(function (e) { print(e.name); });
async function g(a, b = a + 1) { return [a, b].join(","); }
g(1).then(function (v) { print("g", v); });
