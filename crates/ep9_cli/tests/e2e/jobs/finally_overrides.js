// A throw or a rejected promise from the finally callback replaces the result.
Promise.resolve(1).finally(function () { throw new Error("thrown in finally"); }).catch(function (e) { print(e.message); });
Promise.resolve(2).finally(function () { return Promise.reject("rejected in finally"); }).catch(function (e) { print(e); });
var order = [];
Promise.resolve(3).finally(function () {
  return new Promise(function (resolve) { order.push("finally waits"); resolve(); });
}).then(function (v) { order.push("value " + v); print(order.join(", ")); });
