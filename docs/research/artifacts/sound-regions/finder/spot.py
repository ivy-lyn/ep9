import sys, random
sys.path.insert(0, '$SR/finder')
import dbg
repo = sys.argv[1]; seed = int(sys.argv[2]) if len(sys.argv) > 2 else 1
D = dbg.run(repo)
Q, classes, file_of, root_fi, reasons, r4r = D["Q"], D["classes"], D["file_of"], D["root_fi"], D["reasons"], D["r4_reasons"]
esc_iface, closed = D["esc_iface"], D["closed"]
LN = dbg.ns["LN"]
random.seed(seed)
rc = [c for c in classes if classes[c]["fi"] in root_fi and c not in Q]
byrule = {}
for c in rc:
    for r, sub, fi, s, g in reasons[c]:
        byrule.setdefault(r + ":" + sub.split("(")[0], []).append((c, fi, s, sub))
    for sub, f in r4r[False][c]:
        byrule.setdefault("r4:" + sub.split("(")[0], []).append((c, None, None, sub + " field=" + f))
    bad = [i for i in esc_iface.get(c, ()) if not closed.get(i, True)]
    if bad:
        byrule.setdefault("r5:iface-shared", []).append((c, None, None, str(bad[:2])))
for k in sorted(byrule):
    lst = byrule[k]
    for c, fi, s, sub in random.sample(lst, min(2, len(lst))):
        v = classes[c]
        where = ("%s:%d" % (file_of[fi].split("/repos/")[1], LN.line(file_of[fi], s))) if fi is not None else ""
        print("%-28s %-28s %s:%d  %s  %s" % (k, v["name"], file_of[v["fi"]].split("/")[-1], LN.line(file_of[v["fi"]], v["s"]), sub, where))
