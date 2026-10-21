// f(...x) and new F(...x) iterate x with its @@iterator (ArgumentListEvaluation, 13.3.8.1)
// A spread that is not last and a default derived constructor iterate only once (15.7.14 NOTE)
function f() { var s = ""; for (var i = 0; i < arguments.length; i++) s += (i ? "," : "") + String(arguments[i]); return arguments.length + ":" + s; }
function F() { this.n = arguments.length; this.a = arguments[0]; this.b = arguments[1]; }
function t(name, fn) { try { print(name, fn()); } catch (e) { print(name, "threw", e instanceof TypeError); } }
var it = { [Symbol.iterator]() { var i = 0; return { next() { i++; return { done: i > 3, value: { v: i * 10 } }; } }; } };
var plain = { [Symbol.iterator]() { var i = 0; return { next() { i++; return { done: i > 3, value: i * 10 }; } }; } };
t("iterable", () => f(...plain) + " " + f(1, ...plain) + " " + f(...plain, 2));
t("objects", () => { var o = new F(...it); return o.n + " " + o.a.v + " " + o.b.v; });
t("method", () => ({ m: f }).m(...plain));
t("holes", () => f(...[1, , 3]));
t("string", () => f(..."ab\u{1F600}"));
t("array", () => f(...[1, 2, 3]) + " " + f(0, ...[1, 2]));
t("not iterable", () => f(...{}));
t("undefined", () => f(...undefined));
var log = [];
var traced = { [Symbol.iterator]() { log.push("iter"); return { next() { log.push("next"); return { done: true }; } }; } };
t("order", () => { try { var u; u(...traced); } catch (e) { return (e instanceof TypeError) + " " + log.join(","); } });
t("bad result", () => f(...{ [Symbol.iterator]() { return { next() { return 1; } }; } }));
t("bad iterator", () => f(...{ [Symbol.iterator]() { return 1; } }));
Object.defineProperty(Array.prototype, 1, { get() { return "proto"; }, configurable: true });
t("hole from proto", () => f(...[1, , 3]));
delete Array.prototype[1];
var values = Array.prototype[Symbol.iterator];
Array.prototype[Symbol.iterator] = function () { var done = false; return { next() { var d = done; done = true; return { done: d, value: "x" }; } }; };
t("replaced", () => f(...[1, 2, 3]));
t("replaced new", () => new F(...[1, 2, 3]).n);
t("replaced not last", () => f(...[1, 2], 9));
class B { constructor(v, w) { this.v = v; this.w = w; } }
class D extends B {}
t("default ctor", () => { var d = new D(5, 6); return d.v + " " + d.w; });
class E extends B { constructor(...a) { super(...a); } }
t("super spread", () => { var e = new E(5, 6); return e.v + " " + e.w; });
Array.prototype[Symbol.iterator] = values;
var big = [];
for (var i = 0; i < 100; i++) big.push({ i: i });
t("gc", () => { var r = (function () { var s = 0; for (var i = 0; i < arguments.length; i++) s += arguments[i].i; return s; })(...big.values()); return r; });
