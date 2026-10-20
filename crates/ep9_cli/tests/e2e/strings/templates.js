var a = 1, b = "two";
print(`a=${a} b=${b} sum=${a + 2}`);
print(`nested ${`inner ${a + a}`} done`);
print(`multi
line`);
print(`${a}${b}${[1, 2]}${{}}`, `${null} ${undefined} ${true}`);
print(`\${escaped}`, `back\`tick`);
