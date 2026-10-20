var cfg = { db: { host: "h", ports: [1, 2] } };
var ref = cfg.db;
ref.host = "changed";
ref.ports.push(3);
print(cfg.db.host, cfg.db.ports.length, cfg.db.ports.join("-"));
