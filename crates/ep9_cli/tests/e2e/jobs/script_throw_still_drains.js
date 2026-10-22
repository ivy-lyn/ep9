// exit: 1
// stderr: Uncaught Error: script failed
// Jobs queued before an uncaught throw still run.
Promise.resolve("queued before the throw").then(function (v) { print(v); });
print("about to throw");
throw new Error("script failed");
