#!/usr/bin/env python3
"""calprobe: every package version in <probe lock> must also be in <root lock> (the probe
builds the versions that ep9 builds). usage: python3 -I lockcheck.py <probe lock> <root lock>"""
import sys
import tomllib


def versions(path):
    with open(path, "rb") as f:
        lock = tomllib.load(f)
    out = {}
    for p in lock.get("package", []):
        out.setdefault(p["name"], set()).add(p["version"])
    return out


probe, root = versions(sys.argv[1]), versions(sys.argv[2])
bad = [f"{n} {v}" for n, vs in sorted(probe.items()) if n != "calprobe"
       for v in sorted(vs) if v not in root.get(n, set())]
if bad:
    sys.exit("lockcheck: not in the root Cargo.lock: " + ", ".join(bad))
print(f"lockcheck: {sum(len(v) for v in probe.values()) - 1} packages, each at a root version")
