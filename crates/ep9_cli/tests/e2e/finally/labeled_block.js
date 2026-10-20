var log = [];
out: {
  try { log.push("a"); break out; } finally { log.push("f"); }
  log.push("unreached");
}
log.push("end");
print(log.join(" "));
