var log = [];
for (var i = 0; i < 4; i++) {
  try {
    if (i === 1) continue;
    if (i === 3) break;
    log.push("body" + i);
  } finally { log.push("fin" + i); }
}
print(log.join(" "));
