function f() { try { throw new Error("inner"); } catch (e) { e.message += "+rethrown"; throw e; } }
try { f(); } catch (e) { print(e.message); }
try { try { throw 1; } catch (e) { throw e + 1; } } catch (e) { print(e); }
