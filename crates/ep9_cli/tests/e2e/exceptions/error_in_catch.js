function g() { throw new RangeError("from g"); }
try {
  try { throw new Error("first"); } catch (e) { g(); }
} catch (e2) { print(e2.name, e2.message); }
