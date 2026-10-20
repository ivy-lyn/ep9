// exit: 1
// error-at: 5:7
// stderr: Uncaught Error: top level
print("before");
throw new Error("top level");
