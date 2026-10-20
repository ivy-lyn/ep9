function f(n) { var out = []; switch (n) { case 0: out.push(0); case 1: out.push(1); case 2: out.push(2); break; case 3: out.push(3); } return out.join(""); }
print(f(0), f(1), f(2), f(3), f(4) === "");
