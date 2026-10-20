// exit: 1
// stderr: Uncaught Error: second
try { throw new Error("first"); } catch (e) { print("caught", e.message); }
throw new Error("second");
