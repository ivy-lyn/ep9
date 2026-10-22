// A rejection that gets a handler before the queue empties is not reported.
var p = Promise.reject(new Error("late handler"));
Promise.resolve().then(function () {}).then(function () {
  p.catch(function (e) { print("handled:", e.message); });
});
print("sync");
