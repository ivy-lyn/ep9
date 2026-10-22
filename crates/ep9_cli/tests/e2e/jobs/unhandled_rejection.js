// exit: 1
// stderr: Uncaught Error: nobody caught this
// A rejection with no handler when the queue is empty is an uncaught error (exit 1).
print("start");
Promise.reject(new Error("nobody caught this"));
Promise.resolve().then(function () { print("jobs still ran"); });
