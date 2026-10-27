// An async IIFE and a named async function expression.
(async function () { print("iife", await 1); })();
var named = async function self(n) { return n > 0 ? await self(n - 1) + n : 0; };
named(4).then(function (v) { print("named", v); });
