"""Cluster tsrs parser-only failures. usage: cluster.py <manifest> <merged-results> <outdir>"""
import csv, json, os, re, sys, collections

man = {r["id"]: r for r in map(json.loads, open(sys.argv[1]))}
res = {r["id"]: r for r in map(json.loads, open(sys.argv[2]))}
od = sys.argv[3]

def norm(msg):
    if msg is None: return None
    m = re.sub(r"`[^`]*`", "`X`", msg)
    m = re.sub(r"'[^']*'", "'X'", m)
    m = re.sub(r"\"[^\"]*\"", '"X"', m)
    m = re.sub(r"\d+", "N", m)
    return m

def tsrs_p(x): return x["tsrs"]["perr"] + x["tsrs"]["jserr"] > 0

fn_rows = []
for i, m in man.items():
    if m["top"] == "intl402" or not m["expect_error"]: continue
    runs = res[i]["runs"]
    if all(tsrs_p(x) for x in runs): continue
    # the run(s) where tsrs missed the error
    miss = [x for x in runs if not tsrs_p(x)]
    x = miss[0]
    o, t = x["oxc"], x["tsrs"]
    where = "oxc_parser" if o["perr"] else ("oxc_semantic" if o["serr"] else "oxc_none")
    omsg = o["first"] if o["perr"] else o["first_sem"]
    d = m["rel"].split("/")
    fn_rows.append({
        "rel": m["rel"], "dir2": "/".join(d[:2]), "dir3": "/".join(d[:3]), "dir4": "/".join(d[:-1]),
        "missed_modes": "".join(y["mode"] for y in miss), "runs": m["runs"],
        "features": " ".join(m["features"]),
        "oxc_where": where, "oxc_msg": omsg, "oxc_cluster": norm(omsg),
        "tsrs_binder_f": all(y["tsrs"]["bindf"] > 0 for y in miss),
        "tsrs_binder_any": all(y["tsrs"]["bind"] > 0 for y in miss),
        "tsrs_regex": all(y["tsrs"]["rx"] > 0 for y in miss),
        "tsrs_bind_first": t["first_bind"], "tsrs_rx_first": t["first_rx"],
    })

with open(os.path.join(od, "tsrs_false_negatives.csv"), "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(fn_rows[0].keys())); w.writeheader(); w.writerows(fn_rows)

print("total tsrs parser-only false negatives (main scope):", len(fn_rows))
print("by oxc_where:", collections.Counter(r["oxc_where"] for r in fn_rows))
print("caught by tsrs binder(filtered):", sum(r["tsrs_binder_f"] for r in fn_rows),
      " binder(any):", sum(r["tsrs_binder_any"] for r in fn_rows), " regex:", sum(r["tsrs_regex"] for r in fn_rows))

def top(key, n=40):
    c = collections.Counter(r[key] for r in fn_rows)
    out = []
    for k, v in c.most_common(n):
        ex = [r for r in fn_rows if r[key] == k]
        wh = collections.Counter(r["oxc_where"] for r in ex)
        bf = sum(r["tsrs_binder_f"] for r in ex); rx = sum(r["tsrs_regex"] for r in ex)
        feats = collections.Counter(f for r in ex for f in r["features"].split())
        out.append({"key": k, "count": v, "oxc_where": dict(wh), "tsrs_binder_f": bf, "tsrs_regex": rx,
                    "top_features": feats.most_common(3), "examples": [r["rel"] for r in ex[:3]],
                    "oxc_msgs": collections.Counter(r["oxc_cluster"] for r in ex).most_common(3)})
    return out

clusters = {"by_oxc_message": top("oxc_cluster", 60), "by_dir4": top("dir4", 60), "by_dir3": top("dir3", 40)}
json.dump(clusters, open(os.path.join(od, "tsrs_fn_clusters.json"), "w"), indent=1)
for k in ["by_oxc_message"]:
    print("\n==", k)
    for c in clusters[k]:
        print(f"{c['count']:5} {c['oxc_where']} bindF={c['tsrs_binder_f']} rx={c['tsrs_regex']} | {c['key']} | {c['examples'][0]}")
