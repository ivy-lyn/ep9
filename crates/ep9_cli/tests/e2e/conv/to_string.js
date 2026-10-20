print("" + undefined, "" + null, "" + true, "" + 12.5, "" + [1, [2, 3]], "" + {}, "" + [null], "" + [undefined, 1]);
print("" + (function () { return 1; }) === "" ? "empty" : "nonempty");
