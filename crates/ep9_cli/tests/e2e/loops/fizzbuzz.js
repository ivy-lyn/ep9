var out = [];
for (var i = 1; i <= 15; i++) out.push(i % 15 === 0 ? "FizzBuzz" : i % 3 === 0 ? "Fizz" : i % 5 === 0 ? "Buzz" : i);
print(out.join(" "));
