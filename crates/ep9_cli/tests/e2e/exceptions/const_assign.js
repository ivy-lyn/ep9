const c = 1;
try { c = 2; } catch (e) { print(e.name, e.message); }
try { c++; } catch (e) { print(e.name); }
print(c);
