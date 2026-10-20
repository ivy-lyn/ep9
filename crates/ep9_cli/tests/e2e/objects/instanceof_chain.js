function A() {}
function B() {}
B.prototype = new A();
var b = new B();
print(b instanceof B, b instanceof A, b instanceof Object, {} instanceof A);
print([] instanceof Array, [] instanceof Object, (function () {}) instanceof Object);
