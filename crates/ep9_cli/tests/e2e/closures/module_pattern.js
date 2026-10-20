var stack = (function () {
  var items = [];
  return {
    push: function (x) { items.push(x); return this; },
    pop: function () { var x = items[items.length - 1]; items.length = items.length - 1; return x; },
    size: function () { return items.length; },
  };
})();
stack.push(1).push(2).push(3);
print(stack.size(), stack.pop(), stack.pop(), stack.size());
