function spin(n){let s=0;for(let i=0;i<n;i++)s+=Math.sqrt(i);return s}
function fdecl(a, b) { return spin(2e5) + a + b; }
class K {
  x = 1;
  constructor(y) { this.y = y; spin(2e5); }
  meth(a) { return spin(2e5); }
  get g() { return spin(2e5); }
  static st() { return spin(2e5); }
}
class D { x = spin(2e5); }
const arrow = (z) => spin(2e5);
const fexp = function named(q) { return spin(2e5); };
let t=0; for (let i=0;i<300;i++){ t+=fdecl(1,2); const k=new K(i); t+=k.meth(1)+k.g+K.st(); new D(); t+=arrow(1)+fexp(1); }
console.log(t>0);
