try {
  try { throw "first"; } finally { throw "second"; }
} catch (e) { print(e); }
function k() { for (;;) { try { return "loop return"; } finally { break; } } return "after loop"; }
print(k());
