// An async function object inherits from %AsyncFunction.prototype% (§15.8.3, §27.7.3), for every
// syntax form and for the AsyncFunction constructor.
async function af() {}
var AFP = Object.getPrototypeOf(af);
print(AFP === Function.prototype, AFP[Symbol.toStringTag], Object.prototype.toString.call(af));
print(AFP.constructor.name, AFP.constructor.length, Object.getPrototypeOf(AFP) === Function.prototype);
var arrow = async () => 1;
var obj = { async m() {} };
class K { async m() {} static async s() {} }
print([arrow, obj.m, K.prototype.m, K.s, async function () {}].map(f => Object.getPrototypeOf(f) === AFP).join());
var AF = AFP.constructor;
var dyn = new AF("a", "return await a");
print(Object.getPrototypeOf(dyn) === AFP, dyn.name, af instanceof AF, "prototype" in af);
print(Object.getPrototypeOf(function* () {}) === AFP, Object.getPrototypeOf(function () {}) === Function.prototype);
