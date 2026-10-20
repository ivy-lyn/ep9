class A { get_() { return "A"; } }
class B extends A {
  m() { super.field = 5; return this.field; }
  k(key) { return super[key](); }
}
var b = new B();
print(b.m(), b.field, b.k("get_"));
