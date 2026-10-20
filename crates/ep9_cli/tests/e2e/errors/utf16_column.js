// exit: 1
// error-at: 4:20
// stderr: Uncaught RangeError: astral
/* 😀😀 */ var s = new RangeError("astral"); throw s;
