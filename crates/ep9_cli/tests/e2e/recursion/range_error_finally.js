var log = [];
function deep(n) { try { return deep(n + 1); } finally { if (n === 0) log.push("outer finally"); } }
try { deep(0); } catch (e) { log.push(e.name); }
print(log.join(", "));
