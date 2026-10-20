function F() { return new.target === F ? "new" : "call"; }
print(F(), new F() instanceof F);
function G() { this.t = typeof new.target; }
print(new G().t);
