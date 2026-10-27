// An async arrow keeps the `this` and `arguments` of its function after await.
function host() {
  var f = async (x) => { await null; return this.name + ":" + x + ":" + arguments[0]; };
  return f("arrow");
}
host.call({ name: "obj" }, "outer arg").then(print);
var concise = async (a, b) => a * await b;
concise(6, 7).then(print);
