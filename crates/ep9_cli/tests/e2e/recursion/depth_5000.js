function down(n) { return n === 0 ? 0 : 1 + down(n - 1); }
print(down(5000));
function sumTo(n, acc) { return n === 0 ? acc : sumTo(n - 1, acc + n); }
print(sumTo(5000, 0));
