// Promise.all keeps the input order, whatever order the elements settle in.
var late = new Promise(function (resolve) {
  Promise.resolve().then(function () {}).then(function () { resolve("late"); });
});
Promise.all([late, "now", Promise.resolve("soon")]).then(function (values) {
  print(values.length, values.join(","));
});
Promise.all([1, Promise.reject(new Error("one failed")), late]).catch(function (e) {
  print("rejected:", e.message);
});
