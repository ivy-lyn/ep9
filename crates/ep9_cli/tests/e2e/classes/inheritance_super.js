class Animal {
  constructor(name) { this.name = name; }
  speak() { return this.name + " makes a sound"; }
}
class Dog extends Animal {
  constructor(name) { super(name); this.kind = "dog"; }
  speak() { return super.speak() + " (woof)"; }
}
var d = new Dog("Rex");
print(d.speak(), d.kind, d instanceof Dog, d instanceof Animal);
