let a = 1;
{ let a = 2; { let a = 3; print(a); } print(a); }
print(a);
const c = { n: 1 };
c.n = 2;
print(c.n);
