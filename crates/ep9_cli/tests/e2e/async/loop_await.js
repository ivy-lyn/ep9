// await in a loop, with break and continue.
async function f() {
  var out = [];
  for (var i = 0; i < 10; i++) {
    if (i === 2) continue;
    if (i === 6) break;
    out.push(await i);
  }
  return out.join(" ");
}
f().then(print);
