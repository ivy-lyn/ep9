function f() { return f.count = (f.count || 0) + 1; }
f(); f();
print(f.count, typeof f.prototype, f.prototype.constructor === f);
