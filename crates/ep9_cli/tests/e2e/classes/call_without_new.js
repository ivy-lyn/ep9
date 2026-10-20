class K {}
try { K(); } catch (e) { print(e instanceof TypeError); }
