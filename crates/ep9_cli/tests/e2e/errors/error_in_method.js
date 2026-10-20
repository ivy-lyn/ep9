// exit: 1
// error-at: 7:11
// stderr: Uncaught Error: in method
var o = {
  m: function () {
    var x = 1;
    throw new Error("in method");
  },
};
o.m();
