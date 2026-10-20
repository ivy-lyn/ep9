function f() { if (true) { var v = "function scoped"; } return v; }
print(f());
for (var i = 0; i < 3; i++) {}
print(i);
