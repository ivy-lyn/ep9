var nz = -0;
print(nz, 0 === nz, 1 / nz, 1 / 0);
print(1 / (0 * -1), 1 / (-0 + 0), 1 / (-0 - 0), 1 / (0 - 0));
print(1 / Math_neg(0));
function Math_neg(x) { return -x; }
print(1 / (-1 % 1), 1 / (-5 % 5), 1 / (5 % -5));
print(nz + "", String_of(nz), [nz] + "");
function String_of(x) { return `${x}`; }
