var n = 100, sieve = [], count = 0;
for (var i = 2; i <= n; i++) sieve[i] = true;
for (var p = 2; p * p <= n; p++) if (sieve[p]) for (var m = p * p; m <= n; m += p) sieve[m] = false;
for (var q = 2; q <= n; q++) if (sieve[q]) count++;
print(count);
