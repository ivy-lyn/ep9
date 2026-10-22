// Promise.race settles like its first settled element.
var never = new Promise(function () {});
Promise.race([never, Promise.resolve("fast"), "plain"]).then(function (v) { print("race:", v); });
Promise.race([never, Promise.reject(new Error("fail fast"))]).catch(function (e) { print("race error:", e.message); });
Promise.race(["plain first", Promise.resolve("second")]).then(function (v) { print("race:", v); });
