// A recursive async function: each level awaits the next.
async function depth(n) { if (n === 0) return 0; return 1 + await depth(n - 1); }
depth(100).then(function (v) { print("depth", v); });
