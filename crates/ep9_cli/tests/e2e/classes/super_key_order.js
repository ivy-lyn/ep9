// `super[k] = v`: the key is converted (ToPropertyKey) after `v` is evaluated, as for
// `o[k] = v`. A throwing key conversion comes after `v` too.
var log;
function key(name) { return { toString: function () { log.push("K"); return name; } }; }
function rhs(v) { log.push("R"); return v; }
var bad = { toString: function () { log.push("K!"); throw new Error("key"); } };
var base = { x: 1, y: 1, m: function (a) { log.push("M" + a); return this === obj; } };
var obj = {
  __proto__: base,
  assign() { log = []; var r = super[key("x")] = rhs(5); return log.join("") + " " + r + " " + this.x; },
  read() { log = []; var r = super[key("x")]; return log.join("") + " " + r; },
  call() { log = []; var r = super[key("m")](rhs(1)); return log.join("") + " " + r; },
  throwing() {
    log = [];
    try { super[bad] = rhs(5); } catch (e) { log.push(e.message); }
    try { super[bad]; } catch (e) { log.push(e.message); }
    return log.join(",");
  },
  values() {
    // Reads go to the prototype, stores to `this`.
    var k = "y", out = [];
    super[k] = 10;
    out.push(this.y);
    super[k] += 5;
    out.push(this.y);
    super[k]++;
    out.push(this.y);
    out.push(super[k]--, this.y);
    super[k] ??= 9;
    out.push(this.y, base.y);
    return out.join(" ");
  },
};
print("=", obj.assign());
print("read", obj.read());
print("call", obj.call());
print("throwing", obj.throwing());
print("values", obj.values());
class A { get() { return "A"; } }
class B extends A {
  run() { log = []; var r = super[key("v")] = rhs(7); return log.join("") + " " + r + " " + this.v; }
}
print("class", new B().run());
var o = {};
log = [];
o[key("p")] = rhs(1);
print("plain", log.join(""), o.p);
