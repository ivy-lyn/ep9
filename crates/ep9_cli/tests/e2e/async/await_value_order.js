// Values of await: a number, a string, an object, undefined.
async function f() {
  var a = await 1;
  var b = await "two";
  var c = await { x: 3 };
  var d = await undefined;
  print(a, b, c.x, d);
}
f().then(function () { print("done"); });
print("first");
