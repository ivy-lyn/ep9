// exit: 1
// stderr: Uncaught Error: nobody waits
// An async function that rejects with no handler is an uncaught error at the end (exit 1).
async function lost() { await null; throw new Error("nobody waits"); }
lost();
print("still prints");
