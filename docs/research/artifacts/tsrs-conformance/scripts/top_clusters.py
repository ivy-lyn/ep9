"""Top tsrs parser-only failure clusters by directory and by feature. usage: top_clusters.py <outdir>"""
import csv, json, os, sys, collections
od = sys.argv[1]
rows = list(csv.DictReader(open(os.path.join(od, "tsrs_false_negatives_categorized.csv"))))
out = {}
def by(keyf, n=25):
    c = collections.Counter(keyf(r) for r in rows)
    res = []
    for k, v in c.most_common(n):
        ex = [r for r in rows if keyf(r) == k]
        res.append({"cluster": k, "count": v,
                    "categories": dict(collections.Counter(r["cat"] for r in ex)),
                    "labels": collections.Counter(r["label"] for r in ex).most_common(2),
                    "binder_catches": sum(r["tsrs_binder_f"] == "True" for r in ex),
                    "examples": [r["rel"] for r in ex[:2]]})
    return res
out["by_dir"] = by(lambda r: r["dir4"].replace("language/", "L/").replace("built-ins/", "B/"))
feat_rows = []
fc = collections.Counter(f for r in rows for f in (r["features"].split() or ["<none>"]))
out["by_feature"] = [{"feature": k, "count": v} for k, v in fc.most_common(25)]
json.dump(out, open(os.path.join(od, "tsrs_top_clusters.json"), "w"), indent=1)
print("== top 25 dirs (tsrs parser-only false negatives)")
for c in out["by_dir"]:
    print(f"{c['count']:4} {c['cluster']:70} {c['categories']} bindF={c['binder_catches']} | {c['labels'][0][0][:60]} | ex: {c['examples'][0].split('/')[-1]}")
print("== top features")
print(", ".join(f"{x['feature']}:{x['count']}" for x in out["by_feature"]))
