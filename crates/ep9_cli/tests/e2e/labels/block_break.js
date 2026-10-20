var log = [];
blk: {
  log.push("in");
  if (log.length) break blk;
  log.push("skipped");
}
log.push("out");
print(log.join(" "));
