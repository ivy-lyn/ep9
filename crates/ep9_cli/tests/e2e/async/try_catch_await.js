// A rejected await throws at the await; try/catch handles it and the function goes on.
async function f() {
  try { await Promise.reject("r1"); print("not here"); } catch (e) { print("catch", e); }
  try { await Promise.resolve("ok"); print("no throw"); } catch (e) { print("not here"); }
  return "end";
}
f().then(function (v) { print(v); });
