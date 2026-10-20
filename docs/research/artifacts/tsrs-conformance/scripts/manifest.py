"""Build a JSONL manifest of test262 tests with parsed frontmatter.

usage: manifest.py <test262-root> <out.jsonl>
"""
import json, os, re, sys
import yaml

root = os.path.abspath(sys.argv[1])
out = sys.argv[2]
DIRS = ["language", "built-ins", "annexB", "intl402"]
FM = re.compile(r"/\*---(.*?)---\*/", re.S)

rows = []
bad = 0
for d in DIRS:
    base = os.path.join(root, "test", d)
    for dp, dns, fns in os.walk(base):
        dns.sort()
        for fn in sorted(fns):
            if not fn.endswith(".js") or "_FIXTURE" in fn:
                continue
            p = os.path.join(dp, fn)
            with open(p, "rb") as f:
                src = f.read().decode("utf-8", errors="surrogatepass")
            m = FM.search(src)
            meta = {}
            if m:
                try:
                    meta = yaml.safe_load(m.group(1)) or {}
                except Exception as e:
                    bad += 1
                    meta = {"_yaml_error": str(e)}
            flags = meta.get("flags") or []
            neg = meta.get("negative") or None
            phase = neg.get("phase") if isinstance(neg, dict) else None
            etype = neg.get("type") if isinstance(neg, dict) else None
            if "module" in flags:
                runs = "m"
            elif "onlyStrict" in flags:
                runs = "S"
            elif "noStrict" in flags or "raw" in flags:
                runs = "s"
            else:
                runs = "sS"
            rel = os.path.relpath(p, os.path.join(root, "test"))
            rows.append({
                "id": len(rows), "rel": rel, "path": p, "top": d,
                "flags": flags, "phase": phase, "etype": etype,
                "features": meta.get("features") or [],
                "runs": runs,
                "expect_error": phase in ("parse", "early"),
            })
with open(out, "w") as f:
    for r in rows:
        f.write(json.dumps(r) + "\n")
print(f"wrote {len(rows)} tests, yaml errors: {bad}", file=sys.stderr)
