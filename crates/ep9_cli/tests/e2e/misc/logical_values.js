print(1 && 2, 0 && 2, 1 || 2, 0 || 2, null || undefined, "" && x_never);
var calls = 0;
function t() { calls++; return true; }
false && t(); true || t(); true && t(); false || t();
print(calls);
