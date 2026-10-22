// Promise.withResolvers hands out the promise with its two functions.
var r = Promise.withResolvers();
print(typeof r.promise.then, typeof r.resolve, typeof r.reject);
r.promise.then(function (v) { print("resolved later with", v); });
Promise.resolve().then(function () {
  print("resolving now");
  r.resolve("done");
});
var s = Promise.withResolvers();
s.promise.catch(function (e) { print("rejected with", e); });
s.reject("nope");
