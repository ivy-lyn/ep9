function a() { b(); print("unreached a"); }
function b() { c(); print("unreached b"); }
function c() { throw new Error("from c"); }
try { a(); } catch (e) { print("caught:", e.message); }
function wrap(f) { try { f(); return "ok"; } catch (e) { return "wrapped " + e; } }
print(wrap(function () { throw "x"; }), wrap(function () {}));
