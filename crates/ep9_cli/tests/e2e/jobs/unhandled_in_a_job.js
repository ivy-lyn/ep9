// exit: 1
// stderr: Uncaught TypeError: thrown in a job
// A handler that throws with no catch after it leaves a rejected promise behind.
Promise.resolve(1).then(function (v) {
  print("handler", v);
  throw new TypeError("thrown in a job");
});
print("sync");
