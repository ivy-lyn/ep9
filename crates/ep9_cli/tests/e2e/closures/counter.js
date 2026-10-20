function makeCounter() { var n = 0; return function () { n += 1; return n; }; }
var c1 = makeCounter(), c2 = makeCounter();
c1(); c1();
print(c1(), c2(), c1(), c2());
