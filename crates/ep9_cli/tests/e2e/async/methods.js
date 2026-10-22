// Async methods of objects and classes, static and private, with super.
class Base { hello() { return "hello"; } }
class Derived extends Base {
  #secret = 42;
  async greet(name) { await null; return super.hello() + " " + name; }
  static async twice(x) { return 2 * await x; }
  async #hidden() { await null; return this.#secret; }
  reveal() { return this.#hidden(); }
}
var d = new Derived();
d.greet("world").then(print);
Derived.twice(Promise.resolve(21)).then(print);
d.reveal().then(print);
var o = { n: 5, async get() { await null; return this.n; } };
o.get().then(print);
