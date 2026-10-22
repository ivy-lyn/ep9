// await inside expressions: operands, calls, templates, array and object literals.
async function f() {
  var a = (await 1) + (await 2);
  var b = Math.max(await 3, await 4);
  var c = `${await "x"}-${await "y"}`;
  var d = [await 5, await 6];
  var e = { k: await 7 };
  var g = (await 0) ? "yes" : await "no";
  print(a, b, c, d.join(","), e.k, g);
}
f();
