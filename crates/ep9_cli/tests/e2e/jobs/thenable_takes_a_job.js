// Resolving with a thenable calls its then in a job of its own (one extra job).
var thenable = {
  then: function (resolve) {
    print("then called");
    resolve("from thenable");
  },
};
Promise.resolve(thenable).then(function (v) { print("got", v); });
Promise.resolve(1).then(function () { print("tick 1"); }).then(function () { print("tick 2"); }).then(function () { print("tick 3"); });
print("sync");
