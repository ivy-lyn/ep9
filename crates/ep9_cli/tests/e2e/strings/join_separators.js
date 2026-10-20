var a = [1, 2, 3];
print(a.join(), a.join(""), a.join(" - "), a.join(undefined), a.join(null));
print([].join(), [undefined].join(), [null, null].join(), [, 1].join());
