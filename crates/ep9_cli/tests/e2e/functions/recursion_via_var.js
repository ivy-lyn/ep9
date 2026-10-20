var countdown = function (n) { return n === 0 ? "liftoff" : countdown(n - 1); };
print(countdown(10));
