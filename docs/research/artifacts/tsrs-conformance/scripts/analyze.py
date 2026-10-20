"""Score results.jsonl against the manifest. usage: analyze.py <manifest> <results> <outdir>"""
import csv, json, os, sys, collections

man = {r["id"]: r for r in map(json.loads, open(sys.argv[1]))}
res = {r["id"]: r for r in map(json.loads, open(sys.argv[2]))}
od = sys.argv[3]

def t_err(run, v):
    t = run["tsrs"]
    p = t["perr"] + t["jserr"] > 0
    if v == "tsrs_parser": return p
    if v == "tsrs_parser+regex": return p or t["rx"] > 0
    if v == "tsrs_parser+binder": return p or t["bindf"] > 0
    if v == "tsrs_parser+binder+regex": return p or t["bindf"] > 0 or t["rx"] > 0
    if v == "tsrs_parser+binder_unfiltered": return p or t["bind"] > 0
    raise KeyError(v)

def o_err(run, v):
    o = run["oxc"]
    if v == "oxc_parser": return o["perr"] > 0
    if v == "oxc_parser+semantic": return o["perr"] > 0 or o["serr"] > 0
    raise KeyError(v)

VARIANTS = ["tsrs_parser", "tsrs_parser+regex", "tsrs_parser+binder", "tsrs_parser+binder+regex",
            "tsrs_parser+binder_unfiltered", "oxc_parser", "oxc_parser+semantic"]

def errs(run, v):
    return t_err(run, v) if v.startswith("tsrs") else o_err(run, v)

def passed(m, r, v):
    runs = r["runs"]
    if m["expect_error"]:
        return all(errs(x, v) for x in runs)
    return not any(errs(x, v) for x in runs)

def cat(m):
    sign = "neg" if m["expect_error"] else "pos"
    goal = "module" if m["runs"] == "m" else "script"
    return sign, goal

rows = []
stats = collections.defaultdict(lambda: [0, 0])  # key -> [pass, total]
fails = collections.defaultdict(list)
for i, m in man.items():
    r = res[i]
    sign, goal = cat(m)
    scope = "main" if m["top"] != "intl402" else "intl402"
    row = {"id": i, "rel": m["rel"], "top": m["top"], "sign": sign, "goal": goal, "runs": m["runs"],
           "phase": m["phase"] or "", "features": " ".join(m["features"])}
    for v in VARIANTS:
        ok = passed(m, r, v)
        row[v] = int(ok)
        for key in [(v, scope, "all", "all"), (v, scope, sign, "all"), (v, scope, sign, goal), (v, scope, "all", goal),
                    (v, "main+intl402", sign, "all"), (v, scope + ":" + m["top"], sign, "all")]:
            stats[key][1] += 1
            stats[key][0] += ok
        if not ok:
            fails[(v, scope, sign)].append(m["rel"])
    rows.append(row)

os.makedirs(od, exist_ok=True)
with open(os.path.join(od, "per_test.csv"), "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
    w.writeheader(); w.writerows(rows)
with open(os.path.join(od, "summary.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["variant", "scope", "sign", "goal", "passed", "total", "pct"])
    for (v, sc, s, g), (p, t) in sorted(stats.items()):
        w.writerow([v, sc, s, g, p, t, f"{100*p/t:.2f}"])
summ = {f"{v}|{sc}|{s}|{g}": {"passed": p, "total": t, "pct": round(100 * p / t, 2)} for (v, sc, s, g), (p, t) in sorted(stats.items())}
json.dump(summ, open(os.path.join(od, "summary.json"), "w"), indent=1)
fd = os.path.join(od, "failing"); os.makedirs(fd, exist_ok=True)
for (v, sc, s), lst in fails.items():
    with open(os.path.join(fd, f"{v}.{sc}.{s}.txt"), "w") as f:
        f.write("\n".join(sorted(lst)) + "\n")

# console table
print(f"{'variant':32} {'scope':8} {'sign':4} {'goal':6} {'pass':>7}/{'total':<7} pct")
for (v, sc, s, g), (p, t) in sorted(stats.items(), key=lambda x: (x[0][1], x[0][2], x[0][3], VARIANTS.index(x[0][0]))):
    if sc in ("main", "main+intl402") or (sc == "intl402" and s == "all" and g == "all"):
        print(f"{v:32} {sc:8} {s:4} {g:6} {p:7}/{t:<7} {100*p/t:6.2f}")
