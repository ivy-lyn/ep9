// Promise.any takes the first fulfilment, or rejects with an AggregateError of all reasons.
Promise.any([Promise.reject("x"), Promise.resolve("y"), Promise.resolve("z")]).then(function (v) { print("any:", v); });
Promise.any([Promise.reject("r1"), Promise.reject("r2")]).catch(function (e) {
  print(e instanceof AggregateError, e.name, e.message);
  print(e.errors.length, e.errors.join(" "));
});
var made = new AggregateError(["a", "b"], "made", { cause: "why" });
print(made.name, made.message, made.errors.length, made.cause, String(made));
