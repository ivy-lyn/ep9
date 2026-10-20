function f() { try { return 1; } finally { return 2; } }
function g() { try { throw 1; } finally { return "swallowed"; } }
function h() { try { return "try"; } finally { "no return"; } }
print(f(), g(), h());
