try {
  try { throw "a"; } catch (e) { print("inner", e); throw "b"; }
} catch (e) { print("outer", e); }
try {
  try { throw "c"; } finally { print("finally runs"); }
} catch (e) { print("outer got", e); }
