function f(n) { var s = ""; switch (n) { case 1: s += "a"; default: s += "d"; case 2: s += "b"; break; case 3: s += "c"; } return s; }
print(f(1), f(2), f(3), f(9));
