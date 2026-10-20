var log = [];
try {
  try { throw new Error("e1"); }
  finally { log.push("inner finally"); }
} catch (e) { log.push("caught " + e.message); }
finally { log.push("outer finally"); }
print(log.join("; "));
