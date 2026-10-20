var log = [];
function level(n) { try { if (n === 0) throw "bottom"; level(n - 1); } finally { log.push(n); } }
try { level(5); } catch (e) { log.push(e); }
print(log.join(" "));
