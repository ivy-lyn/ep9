function Builder() { this.parts = []; }
Builder.prototype.add = function (p) { this.parts.push(p); return this; };
Builder.prototype.build = function () { return this.parts.join(" "); };
print(new Builder().add("a").add("b").add("c").build());
