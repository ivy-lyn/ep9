# Annotate regions.json with the oxc 0.155 node kinds that have the same (start,end) spans.
import json, os, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
J = sys.argv[1]
R = json.load(open(os.path.join(HERE, "regions.json")))
files = sorted({f["file"] for r in R["repos"].values() for f in r["qualifying_functions"]} | {c["file"] for r in R["repos"].values() for c in r["qualifying_classes"]})
lst = os.path.join(HERE, "out", "region_files.txt")
open(lst, "w").write("\n".join(files) + "\n")
dump = os.path.join(HERE, "out", "oxc_regions.jsonl")
subprocess.run([J, dump, "-"], stdin=open(lst), check=True, stderr=subprocess.DEVNULL)
ox = {}
for l in open(dump):
    d = json.loads(l)
    ox[d["file"]] = {(n[1], n[2]): n[0] for n in d["nodes"]}
n = hit = bhit = 0
for r in R["repos"].values():
    for f in r["qualifying_functions"]:
        sp = ox.get(f["file"], {})
        f["oxc_node"] = sp.get((f["start"], f["end"]))
        f["oxc_body"] = sp.get((f["body_start"], f["body_end"]))
        n += 1; hit += f["oxc_node"] is not None; bhit += f["oxc_body"] is not None
    for c in r["qualifying_classes"]:
        sp = ox.get(c["file"], {})
        c["oxc_node"] = sp.get((c["start"], c["end"]))
R["meta"]["oxc_join"] = "oxc_parser 0.155 (R13 oxc_span_dump): %d/%d node spans and %d/%d body spans match an oxc node exactly. Exported functions start at `export` (oxc ExportNamedDeclaration); use body_start/body_end for the FunctionBody." % (hit, n, bhit, n)
json.dump(R, open(os.path.join(HERE, "regions.json"), "w"), indent=1)
print(R["meta"]["oxc_join"])
