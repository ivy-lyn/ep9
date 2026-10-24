// Promise.race between async functions: the one with fewer awaits wins.
async function slow() { await null; await null; await null; return "slow"; }
async function fast() { await null; return "fast"; }
Promise.race([slow(), fast()]).then(function (v) { print("winner", v); });
