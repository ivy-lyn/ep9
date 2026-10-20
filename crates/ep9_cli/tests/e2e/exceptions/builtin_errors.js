try { null.x; } catch (e) { print(e instanceof TypeError); }
try { undefined.x = 1; } catch (e) { print(e instanceof TypeError); }
try { notDefinedAnywhere; } catch (e) { print(e instanceof ReferenceError, e.message); }
try { var f = 5; f(); } catch (e) { print(e instanceof TypeError); }
try { new (function () {}.x)(); } catch (e) { print(e instanceof TypeError); }
try { ({}).method(); } catch (e) { print(e instanceof TypeError); }
