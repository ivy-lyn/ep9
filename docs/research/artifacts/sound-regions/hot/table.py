import json, sys, os
H = os.path.dirname(os.path.abspath(__file__))
names = ["tsc", "awfy-richards", "awfy-deltablue", "awfy-nbody", "vscode-diff", "hono-fetch", "hono-routers", "zod-parse", "datefns-fmtparse"]
print("| workload | mode | samples | Q-v0 self+bi | Q-v3 self+bi | not-Q (analyzed) | other JS (node/eval/driver) | VM gc+program | Q-v0 incl (any frame) | v0 % of analyzed | static fn share v0/v3 |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
for n in names:
    for m in ("", "-noinl"):
        r = json.load(open(os.path.join(H, "res", n + m + ".json")))
        b = r["breakdown_pct"]
        q0 = b.get("Q-v0", 0) + b.get("class-init-Q", 0)
        q3 = q0 + b.get("Q-v3-only", 0)
        nq = b.get("not-Q", 0) + b.get("class-init-not-Q", 0)
        vm = b.get("(garbage collector)", 0) + b.get("(program)", 0) + b.get("(idle)", 0)
        other = round(100 - q3 - nq - vm, 1)
        a = r["pct_of_analyzed_self_plus_builtins"]
        print("| %s | %s | %d | %.1f | %.1f | %.1f | %.1f | %.1f | %.1f | %s | %s/%s |" % (n, "no-inline" if m else "default", r["samples"], q0, q3, nq, other, vm, r["pct_of_all_samples"]["inclusive_v0"], a["v0"], r["static_fn_share"]["v0"], r["static_fn_share"]["v3"]))
