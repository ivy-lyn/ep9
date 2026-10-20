print((function () { return 1; })(), (function () { return 2; }()), (() => 3)());
var r = !function () { return false; }();
print(r);
