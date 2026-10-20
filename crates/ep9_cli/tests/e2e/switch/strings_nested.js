function cls(a, b) {
  switch (a) {
    case "x":
      switch (b) { case "y": return "xy"; default: return "x?"; }
    case "z": return "z";
  }
  return "none";
}
print(cls("x", "y"), cls("x", "q"), cls("z"), cls("w"));
