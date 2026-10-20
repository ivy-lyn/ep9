// IteratorClose: with a throw completion the original exception wins over errors from
// `return`; with break the errors propagate and the result must be an object. v0 has only
// array iterators, so `return` comes from Object.prototype here (no getter: no accessors yet).
var log = [];
Object.prototype.return = function () { log.push("return"); throw new Error("return"); };
try {
  for (var x of [1, 2]) { throw new Error("body"); }
} catch (e) {
  log.push(e.message);
}
try {
  for (var x of [1, 2]) { break; }
} catch (e) {
  log.push(e.message);
}
try {
  var [a = (function () { throw new Error("default"); })()] = [undefined];
} catch (e) {
  log.push(e.message);
}
Object.prototype.return = 5;
try {
  for (var x of [1, 2]) { throw new Error("body2"); }
} catch (e) {
  log.push(e.message);
}
try {
  for (var x of [1, 2]) { break; }
} catch (e) {
  log.push(e.constructor === TypeError);
}
Object.prototype.return = function () { log.push("ret-ok"); return {}; };
for (var x of [1, 2]) { break; }
Object.prototype.return = function () { return 1; };
try {
  for (var x of [1, 2]) { break; }
} catch (e) {
  log.push(e.constructor === TypeError);
}
delete Object.prototype.return;
print(log.join(","));
