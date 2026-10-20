var a = [5, 3, 8, 1, 9, 2, 7, 4, 6, 0];
for (var i = 0; i < a.length; i++) for (var j = 0; j < a.length - 1 - i; j++) if (a[j] > a[j + 1]) { var t = a[j]; a[j] = a[j + 1]; a[j + 1] = t; }
print(a.join(""));
var people = [{ n: "b", age: 30 }, { n: "a", age: 25 }, { n: "c", age: 35 }];
for (var x = 0; x < people.length; x++) for (var y = x + 1; y < people.length; y++) if (people[y].age < people[x].age) { var tmp = people[x]; people[x] = people[y]; people[y] = tmp; }
var names = [];
for (var z = 0; z < people.length; z++) names.push(people[z].n);
print(names.join(","));
