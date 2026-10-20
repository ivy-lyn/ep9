class A { constructor() { this.made = new.target.name; } }
class B extends A {}
class C extends B { constructor() { super(); this.c = true; } }
print(new A().made, new B().made, new C().made, new C() instanceof C, new C() instanceof A);
function Plain() { return new.target === Plain; }
print(new Plain() instanceof Plain, Plain());
