var primes = [];
for (var n = 2; primes.length < 25; n++) {
  var ok = true;
  for (var i = 0; i < primes.length && primes[i] * primes[i] <= n; i++) {
    if (n % primes[i] === 0) { ok = false; break; }
  }
  if (ok) primes.push(n);
}
print(primes.join(" "));
