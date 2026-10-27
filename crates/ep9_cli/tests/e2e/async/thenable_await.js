// Awaiting a thenable calls its `then` in a job.
var thenable = { then: function (resolve) { print("then called"); resolve("from thenable"); } };
async function f() { print("before"); var v = await thenable; print(v); }
f();
print("sync end");
Promise.resolve().then(function () { print("job 1"); }).then(function () { print("job 2"); });
