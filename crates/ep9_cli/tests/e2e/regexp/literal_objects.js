// RegExp literals (M4-P2 RX0A): each evaluation is a new object with %RegExp.prototype%
// and an own lastIndex of 0.
var a = /x/gi, b = /x/gi;
print(a === b, Object.getPrototypeOf(a) === Object.getPrototypeOf(/y/));
print(Object.getPrototypeOf(Object.getPrototypeOf(a)) === Object.prototype);
print(a.lastIndex, Object.getOwnPropertyNames(a).join());
var d = Object.getOwnPropertyDescriptor(a, "lastIndex");
print(d.value, d.writable, d.enumerable, d.configurable);
a.lastIndex = 5;
print(a.lastIndex, b.lastIndex);
function f() { return /re/; }
print(f() !== f(), a instanceof Object, typeof Object.getPrototypeOf(a));
var all = [/[a-z]+/u, /\d{2,}/y, /(?<n>.)\k<n>/, /[\p{L}--[a-z]]/v, /\u{1F600}/u, /./dgimsy];
print(all.length, all.every(function (r) { return typeof r === "object"; }));
