// Number::toString picks the even digit string when x lies exactly halfway between two
// shortest ones (ECMA-262 §6.1.6.1.20 step 5), as node does; JSON.stringify prints the same.
var xs = [900719925474099.25, 0.18521499633789062, 1.8236160278320312, 0.12387466430664062,
  104.47189331054688, 156512588.54492188, 1071568155368862.8, 0.7446975708007812,
  249274520119823.38, 0.5, 2.5e-7, 1.5, 0.3];
for (var i = 0; i < xs.length; i++) print(xs[i], -xs[i]);
print(JSON.stringify(xs));
