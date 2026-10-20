function ack(m, n) { return m === 0 ? n + 1 : n === 0 ? ack(m - 1, 1) : ack(m - 1, ack(m, n - 1)); }
print(ack(0, 0), ack(1, 2), ack(2, 3), ack(3, 3));
