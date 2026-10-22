// A job that queues jobs puts them at the end of the queue.
var log = [];
function step(name, depth) {
  return function () {
    log.push(name + depth);
    if (depth < 2) Promise.resolve().then(step(name, depth + 1));
  };
}
Promise.resolve().then(step("x", 0));
Promise.resolve().then(step("y", 0));
Promise.resolve().then(function () {}).then(function () {}).then(function () {}).then(function () {
  print(log.join(" "));
});
