// exit: 1
// stderr: Uncaught Error: after output
for (var i = 0; i < 3; i++) print("line", i);
throw new Error("after output");
