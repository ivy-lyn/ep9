var a = [1, 2, 3, 4, 5];
a.length = 2;
print(a.join(), a.length, a[3]);
a.length = 4;
print(a.join(), a.length);
a.length = 0;
print(a.length, a.join() === "");
