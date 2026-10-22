// A catch handler recovers the chain; a throw in a handler rejects the next promise.
Promise.reject(new TypeError("bad"))
  .catch(function (e) { print("caught", e.name, e.message); return "recovered"; })
  .then(function (v) { print("then", v); throw new RangeError("again"); })
  .then(function () { print("skipped"); })
  .catch(function (e) { print("caught", e.name, e.message); });
Promise.resolve(5).then(undefined, function () { print("not called"); }).then(function (v) { print("passed through", v); });
