var obj = {
  v: 42,
  get: function () { var arrow = () => this.v; return arrow(); },
  nested: function () { return (() => () => this.v)()(); },
};
print(obj.get(), obj.nested());
var counter = { n: 0, incAll: function (xs) { for (var i = 0; i < xs.length; i++) ((x) => { this.n += x; })(xs[i]); return this.n; } };
print(counter.incAll([1, 2, 3]));
