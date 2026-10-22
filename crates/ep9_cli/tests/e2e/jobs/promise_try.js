// Promise.try calls the function at once with the extra arguments; a throw becomes a rejection.
var p = Promise.try(function (a, b) {
  print("called with", a, b);
  return a * b;
}, 6, 7);
print("after try");
p.then(function (v) { print("value", v); });
Promise.try(function () { throw new Error("sync throw"); }).catch(function (e) { print("rejection", e.message); });
