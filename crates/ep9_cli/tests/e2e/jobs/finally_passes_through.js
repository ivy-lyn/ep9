// finally runs its callback with no argument and passes the value or reason on.
Promise.resolve("kept").finally(function () {
  print("finally args", arguments.length);
  return "dropped";
}).then(function (v) { print("then got", v); });
Promise.reject(new Error("still rejected")).finally(function () {
  print("finally on reject");
}).catch(function (e) { print("catch got", e.message); });
