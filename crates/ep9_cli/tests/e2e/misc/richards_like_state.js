function Task(id, pri) { this.id = id; this.pri = pri; this.runs = 0; }
Task.prototype.run = function (t) { this.runs++; return (t * this.pri + this.id) % 7; };
var tasks = [new Task(1, 3), new Task(2, 5), new Task(3, 2)];
var acc = 0;
for (var t = 0; t < 1000; t++) { var task = tasks[t % 3]; acc = (acc + task.run(t)) % 1000; }
print(acc, tasks[0].runs, tasks[1].runs, tasks[2].runs);
