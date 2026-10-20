var digits = "0123456789";
function toNum(s) { var n = 0; for (var i = 0; i < s.length; i++) { n = n * 10; for (var d = 0; d < 10; d++) if (digits[d] === s[i]) n += d; } return n; }
print(toNum("12345") + 1, toNum("0"), toNum("999") * 2);
