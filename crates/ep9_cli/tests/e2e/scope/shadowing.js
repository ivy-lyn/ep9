var x = "global";
function f() { var x = "local"; function g() { return x; } return g(); }
function h(x) { return x; }
print(f(), h("param"), x);
