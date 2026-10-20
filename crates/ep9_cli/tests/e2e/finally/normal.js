var log = [];
try { log.push("try"); } finally { log.push("finally"); }
log.push("after");
print(log.join(" "));
