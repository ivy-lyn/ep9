var a = [];
for (let i = 0; i < 3; i++) a[i] = () => i * 2;
print(a[0](), a[1](), a[2]());
