// A long chain that allocates in every job, with collections in between.
var p = Promise.resolve({ n: 0, junk: [] });
for (var i = 0; i < 200; i++) {
  p = p.then(function (o) {
    if (o.n % 50 === 0) gc();
    return { n: o.n + 1, junk: [o.n, "s" + o.n, { k: o.n }] };
  });
}
p.then(function (o) { print("steps", o.n, o.junk[1]); });
var all = [];
for (var j = 0; j < 50; j++) all.push(Promise.resolve("e" + j));
gc();
Promise.all(all).then(function (v) { print("all", v.length, v[0], v[49]); });
