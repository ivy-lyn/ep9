function sloppyThis() { return this === globalThis; }
function strictThis() { "use strict"; return this; }
print(sloppyThis(), strictThis(), ({ m: strictThis }).m() !== undefined);
