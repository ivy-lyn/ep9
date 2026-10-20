var log = [];
function thrower() { try { throw "t"; } finally { log.push("f1"); } }
function mid() { try { thrower(); } finally { log.push("f2"); } }
try { mid(); } catch (e) { log.push("c:" + e); }
print(log.join(" "));
