var xs = [];
for (var i = 0; i < 100; i++) xs.push(i * 3);
function bs(t) { var lo = 0, hi = xs.length - 1; while (lo <= hi) { var mid = (lo + hi) >> 1; if (xs[mid] === t) return mid; if (xs[mid] < t) lo = mid + 1; else hi = mid - 1; } return -1; }
print(bs(0), bs(150), bs(297), bs(298), bs(-3));
