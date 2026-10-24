// A throw before or after await rejects the promise; the caller never sees a sync throw.
async function early() { throw new Error("early"); }
async function late() { await 0; throw new Error("late"); }
var p = early();
print("no sync throw", typeof p);
p.catch(function (e) { print("caught", e.message); });
late().catch(function (e) { print("caught", e.message); });
