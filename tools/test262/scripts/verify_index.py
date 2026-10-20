"""Check `ep9-test262 index` output against PyYAML on every test file.

usage:
  cargo run --release -p ep9_test262 -- index --out /tmp/index.json
  uv run --no-project --with pyyaml python3 tools/test262/scripts/verify_index.py \
      third_party/test262 /tmp/index.json

Exit 0 only if every *.js file under test/ (except *_FIXTURE*) is in the index and its flags,
includes, features, locale and negative equal what PyYAML reads. Front matter text: test262's
own regex (tools/packaging/parseTestRecord.py, greedy) and the lazy form of test262-harness;
the script counts files where the two differ.
"""
import json, os, re, sys, yaml

root, index_path = sys.argv[1], sys.argv[2]
GREEDY = re.compile(r"/\*---(.*)---\*/", re.S)
LAZY = re.compile(r"/\*---(.*?)---\*/", re.S)
index = json.load(open(index_path, encoding="utf-8"))
by_path = {t["path"]: t for t in index["tests"]}
assert len(by_path) == index["count"] == len(index["tests"]), "duplicate paths in index"

seen, mismatch, greedy_diff = 0, [], 0
for dp, dns, fns in os.walk(os.path.join(root, "test")):
    for fn in fns:
        if not fn.endswith(".js") or "_FIXTURE" in fn:
            continue
        p = os.path.join(dp, fn)
        rel = os.path.relpath(p, root).replace(os.sep, "/")
        src = open(p, "rb").read().decode("utf-8")
        g, l = GREEDY.search(src), LAZY.search(src)
        if g.group(1) != l.group(1):
            greedy_diff += 1
        meta = yaml.safe_load(l.group(1)) or {}
        want = {
            "flags": meta.get("flags") or [],
            "includes": meta.get("includes") or [],
            "features": meta.get("features") or [],
            "locale": meta.get("locale") or [],
            "negative": None if meta.get("negative") is None else
                {"phase": meta["negative"].get("phase"), "type": meta["negative"].get("type")},
        }
        for k in ("flags", "includes", "features", "locale"):
            assert isinstance(want[k], list) and all(isinstance(x, str) for x in want[k]), (rel, k, want[k])
        if want["negative"] is not None:
            assert set(meta["negative"]) == {"phase", "type"}, (rel, meta["negative"])
        seen += 1
        got = by_path.pop(rel, None)
        if got is None:
            mismatch.append((rel, "missing from index"))
            continue
        got = {k: got.get(k) for k in want}
        if got != want:
            mismatch.append((rel, got, want))

print(f"files checked: {seen}; index entries: {index['count']}; index errors: {index['errors']}")
print(f"index entries with no file: {len(by_path)}")
print(f"greedy vs lazy front matter text differs: {greedy_diff}")
print(f"mismatches: {len(mismatch)}")
for m in mismatch[:20]:
    print("  ", m)
neg = sum(1 for t in index["tests"] if t.get("negative"))
print(f"negative entries in index: {neg}")
sys.exit(1 if mismatch or by_path or index["errors"] else 0)
