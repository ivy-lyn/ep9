// then on a subclass instance makes a subclass instance (through @@species).
class Tracked extends Promise {
  constructor(executor) {
    super(executor);
    Tracked.made++;
  }
}
Tracked.made = 0;
var t = new Tracked(function (resolve) { resolve(1); });
var next = t.then(function (v) { return v + 1; });
print(next instanceof Tracked, next instanceof Promise, Tracked.made);
next.then(function (v) { print("value", v, "made", Tracked.made); });
print(Tracked.resolve(3) instanceof Tracked, Promise.resolve(t) === t, Tracked.resolve(t) === t);
