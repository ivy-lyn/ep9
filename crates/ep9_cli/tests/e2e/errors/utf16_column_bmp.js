// exit: 1
// error-at: 5:17
// stderr: Uncaught Error: bmp
var é = 1;
/* ééé */ throw new Error("bmp");
