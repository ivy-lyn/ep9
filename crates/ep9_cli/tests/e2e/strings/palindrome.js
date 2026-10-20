function isPal(s) { for (var i = 0, j = s.length - 1; i < j; i++, j--) if (s[i] !== s[j]) return false; return true; }
print(isPal("racecar"), isPal("abba"), isPal("abc"), isPal(""), isPal("a"));
