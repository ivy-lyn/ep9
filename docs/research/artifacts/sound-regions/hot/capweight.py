import json, os, sys
H = os.path.dirname(os.path.abspath(__file__))
work = [("tsc-noinl", "typescript"), ("tsc", "typescript"), ("awfy-richards-noinl", "awfy"), ("awfy-deltablue-noinl", "awfy"), ("awfy-nbody-noinl", "awfy"),
        ("hono-routers-noinl", "hono"), ("hono-fetch-noinl", "hono"), ("vscode-diff-noinl", "vscode"), ("vscode-diff", "vscode"), ("datefns-fmtparse-noinl", "datefns"), ("datefns-fmtparse", "datefns")]
print("| workload | Q-v0 self+bi % | no capture | typed captures only | + untyped const captures | untyped mutable capture | Q-v0 with 0 params and 0 locals |")
print("|---|---|---|---|---|---|---|")
out = {}
for w, repo in work:
    r = json.load(open(os.path.join(H, "res", w + ".json")))
    cap = {c["id"]: c for c in json.load(open(os.path.join(H, "cap", repo + ".json")))}
    allf = {f["id"]: f for f in json.load(open(os.path.join(H, "allfuncs", repo + ".allfuncs.json")))}
    tot = r["samples"]
    b = {"none": 0, "typed": 0, "const_untyped": 0, "mut_untyped": 0, "missing": 0}
    triv = 0
    q = 0
    for f in r["per_function"]:
        if not f["v0"] or not f["selfb"]:
            continue
        q += f["selfb"]
        c = cap.get(f["id"])
        af = allf[f["id"]]
        # 0 params and 0 locals in the finder facts
        if af.get("tier0") == 0 and not af["v0_reasons"]:
            pass
        if c is None or "error" in c:
            b["missing"] += f["selfb"]
            continue
        if c["captures"] == 0:
            b["none"] += f["selfb"]
        elif c["untyped"] == 0:
            b["typed"] += f["selfb"]
        elif c["untyped_mutable"] == 0:
            b["const_untyped"] += f["selfb"]
        else:
            b["mut_untyped"] += f["selfb"]
    # trivial: need params/locals count from facts
    out[w] = {k: round(100.0 * v / tot, 1) for k, v in b.items()}
    out[w]["q"] = round(100.0 * q / tot, 1)
    print("| %s | %.1f | %.1f | %.1f | %.1f | %.1f | %s |" % (w, out[w]["q"], out[w]["none"], out[w]["typed"], out[w]["const_untyped"], out[w]["mut_untyped"], "-"))
json.dump(out, open(os.path.join(H, "res", "capweight.json"), "w"), indent=1)
