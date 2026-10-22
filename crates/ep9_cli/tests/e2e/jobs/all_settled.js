// Promise.allSettled reports each result as a status record.
Promise.allSettled([Promise.resolve(1), Promise.reject("no"), 3]).then(function (results) {
  for (var i = 0; i < results.length; i++) {
    var r = results[i];
    print(i, r.status, r.status === "fulfilled" ? r.value : r.reason);
  }
});
