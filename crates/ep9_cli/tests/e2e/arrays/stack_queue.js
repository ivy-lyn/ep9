var st = [];
for (var i = 0; i < 5; i++) st.push(i * i);
var popped = [];
while (st.length > 0) { popped.push(st[st.length - 1]); st.length--; }
print(popped.join(","));
