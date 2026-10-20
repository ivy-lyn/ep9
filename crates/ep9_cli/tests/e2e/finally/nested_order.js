var log = [];
function f() {
  try {
    try { try { return "r"; } finally { log.push(1); } }
    finally { log.push(2); }
  } finally { log.push(3); }
}
print(f(), log.join(","));
