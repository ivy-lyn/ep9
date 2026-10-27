// A store to a private method throws a TypeError with node v24's text, which names the private
// name (PrivateSet step 5; ABI 5 runtime id 45 ThrowTypeError).
class A { #m() {} static t(o) { try { o.#m = 1; } catch (e) { print(e.constructor.name + ": " + e.message); } } }
A.t(new A());
class B { #m() {} static t(o) { try { o.#m += 1; } catch (e) { print(e.message); } } }
B.t(new B());
class C { static #sm() {} static t() { try { C.#sm = 1; } catch (e) { print(e.message); } } }
C.t();
class D { #méthode() {} static t(o) { try { o.#méthode ??= 1; o.#méthode &&= 1; } catch (e) { print(e.message); } } }
D.t(new D());
class E { #m() {} static t(o) { try { o.#m++; } catch (e) { print(e.message); } try { [o.#m] = [1]; } catch (e) { print(e.message); } try { ({ a: o.#m } = { a: 1 }); } catch (e) { print(e.message); } try { for (o.#m of [1]); } catch (e) { print(e.message); } } }
E.t(new E());
class F { #m() {} static t(o) { try { o.#m = 1; } catch (e) { print(e.constructor.name + ": " + e.message); } } }
F.t({});
class G { #\u006d() {} static t(o) { try { o.#m = 1; } catch (e) { print(e.message); } } }
G.t(new G());
