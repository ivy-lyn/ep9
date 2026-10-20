class A { v() { return "A"; } }
class B extends A { v() { return "B" + super.v(); } }
class C extends B { v() { return "C" + super.v(); } }
print(new C().v());
