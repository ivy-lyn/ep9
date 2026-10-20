var id = (function () { var next = 100; return function () { return next++; }; })();
print(id(), id(), id());
var x = 1;
(function () { var x = 2; print(x); })();
print(x);
