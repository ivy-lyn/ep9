var graph = { a: ["b", "c"], b: ["d"], c: ["d", "e"], d: ["f"], e: ["f"], f: [] };
var seen = { a: true }, order = [], queue = ["a"], head = 0;
while (head < queue.length) {
  var n = queue[head++];
  order.push(n);
  var adj = graph[n];
  for (var i = 0; i < adj.length; i++) if (!seen[adj[i]]) { seen[adj[i]] = true; queue.push(adj[i]); }
}
print(order.join(""));
