try { print(x); let x = 1; } catch (e) { print(e.name, e.message); }
try { y = 2; const y = 3; } catch (e) { print(e.name); }
function f() { return z; }
try { f(); } catch (e) { print("tdz via call", e.name); }
let z = "now";
print(f());
