// exit: 1
// stderr: Uncaught RangeError: Maximum call stack size exceeded
print("before");
function f(n) { return f(n + 1) + 1; }
f(0);
print("never");
