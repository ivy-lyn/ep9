try { throw 42; } catch (e) { print(e, typeof e); }
try { throw "str"; } catch (e) { print(e); }
try { throw null; } catch (e) { print(e); }
try { throw undefined; } catch (e) { print(e); }
try { throw { code: 7 }; } catch (e) { print(e.code); }
