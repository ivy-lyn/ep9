// A promise resolved with a native promise settles two jobs later than one resolved with a value.
var inner = Promise.resolve("inner");
new Promise(function (resolve) { resolve(inner); }).then(function (v) { print("outer got", v); });
Promise.resolve("plain").then(function (v) { print(v, 1); }).then(function () { print("plain", 2); }).then(function () { print("plain", 3); });
