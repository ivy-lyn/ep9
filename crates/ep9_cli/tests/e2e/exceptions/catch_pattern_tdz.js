// A catch pattern's names are in TDZ until the pattern binds them.
function check(label, f) {
  try {
    print(label, f());
  } catch (e) {
    print(label, e.name, e.message);
  }
}
check("self", function () {
  try { throw {}; } catch ({ a = a }) { return a; }
});
check("forward", function () {
  try { throw [undefined, 2]; } catch ([a = b, b]) { return a + b; }
});
check("arrow", function () {
  try { throw {}; } catch ({ a = (() => a)() }) { return a; }
});
check("computed key", function () {
  try { throw { k: 1 }; } catch ({ [k]: k }) { return k; }
});
check("nested default", function () {
  try { throw [undefined]; } catch ([{ a } = { a: a }]) { return a; }
});
// Each run of the catch starts in TDZ: the last run's value does not leak.
var seen = [];
for (var i = 0; i < 3; i++) {
  try {
    try {
      throw i === 1 ? {} : { a: "run" + i };
    } catch ({ a = a }) {
      seen.push(a);
    }
  } catch (e) {
    seen.push(e.name);
  }
}
print(seen.join(","));
// The same with the name in a context (a closure captures it).
seen = [];
for (var j = 0; j < 3; j++) {
  try {
    try {
      throw j === 1 ? {} : { a: "ctx" + j };
    } catch ({ a = a }) {
      seen.push((() => a)());
    }
  } catch (e) {
    seen.push(e.name);
  }
}
print(seen.join(","));
// Reads after the pattern, and closures called after it, see the values.
check("after", function () {
  try { throw [1]; } catch ([a, b = a + 1]) { var get = () => a + b; return a + b + get(); }
});
check("simple", function () {
  try { throw 5; } catch (e) { return e; }
});
