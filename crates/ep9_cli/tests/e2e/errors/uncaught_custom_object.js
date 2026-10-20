// exit: 1
// stderr: Uncaught custom toString
throw { toString: function () { return "custom toString"; } };
