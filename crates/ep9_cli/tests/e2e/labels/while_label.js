var n = 0;
loop: while (true) {
  n++;
  switch (n) { case 3: break loop; default: continue loop; }
}
print(n);
