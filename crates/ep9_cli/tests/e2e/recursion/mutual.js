function isEven(n) { return n === 0 ? true : isOdd(n - 1); }
function isOdd(n) { return n === 0 ? false : isEven(n - 1); }
print(isEven(10), isOdd(7), isEven(1001), isOdd(0));
