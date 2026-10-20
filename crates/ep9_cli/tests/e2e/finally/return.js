var log = [];
function f() { try { log.push("try"); return "ret"; } finally { log.push("finally"); } }
print(f(), log.join(" "));
