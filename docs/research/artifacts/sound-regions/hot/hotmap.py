#!/usr/bin/env python3
"""Weight sound-region finder verdicts by CPU time.

Usage: python3 -I hotmap.py <label> <repo>[,<repo2>] <out.json> <profile.cpuprofile>...

- Reads V8 .cpuprofile files (node --cpu-prof).
- Maps each JS frame (function start position, 0-based line/col) through the generated file's
  source map (linked //# sourceMappingURL) back to the original TS file and position.
- Finds the innermost finder function (allfuncs/<repo>.allfuncs.json) whose span [s, e) holds
  the original position (spans: UTF-8 byte offsets after BOM strip).
- Classifies every sample by its leaf frame and by its stack.
"""
import bisect
import json
import os
import sys
from collections import Counter, defaultdict

HOT = os.path.dirname(os.path.abspath(__file__))
FINDER = os.path.join(os.path.dirname(HOT), "finder")

VM_FRAMES = {"(garbage collector)", "(program)", "(idle)", "(root)"}


# ---------------------------------------------------------------- source maps
B64 = {c: i for i, c in enumerate("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/")}


def vlq_decode(seg):
    out, val, shift = [], 0, 0
    for ch in seg:
        d = B64[ch]
        cont = d & 32
        d &= 31
        val += d << shift
        if cont:
            shift += 5
        else:
            neg = val & 1
            val >>= 1
            out.append(-val if neg else val)
            val, shift = 0, 0
    return out


class SourceMap:
    def __init__(self, path):
        m = json.load(open(path))
        base = os.path.dirname(path)
        root = m.get("sourceRoot") or ""
        self.sources = [os.path.normpath(os.path.join(base, root, s)) for s in m["sources"]]
        self.lines = []  # per generated line: sorted list of (gcol, src, oline, ocol)
        src = oline = ocol = 0
        for line in m["mappings"].split(";"):
            gcol = 0
            segs = []
            if line:
                for seg in line.split(","):
                    if not seg:
                        continue
                    v = vlq_decode(seg)
                    gcol += v[0]
                    if len(v) >= 4:
                        src += v[1]
                        oline += v[2]
                        ocol += v[3]
                        segs.append((gcol, src, oline, ocol))
            self.lines.append(segs)

    def lookup(self, line, col):
        if line >= len(self.lines):
            return None
        segs = self.lines[line]
        if not segs:
            return None
        i = bisect.bisect_right(segs, (col, 1 << 30, 0, 0)) - 1
        if i < 0:
            # use first mapping on the line (function start before first mapping: rare)
            i = 0
        g, s, ol, oc = segs[i]
        return self.sources[s], ol, oc


SM_CACHE = {}


def smap_for(url):
    if url in SM_CACHE:
        return SM_CACHE[url]
    sm = None
    if url.startswith("file://"):
        p = url[7:]
        mp = p + ".map"
        if os.path.exists(mp):
            sm = SourceMap(mp)
    SM_CACHE[url] = sm
    return sm


# ---------------------------------------------------------------- original text offsets
TXT = {}


def byte_offset(path, line, col):
    """0-based line/col (UTF-16 units) -> UTF-8 byte offset after BOM strip"""
    if path not in TXT:
        try:
            b = open(path, "rb").read()
        except OSError:
            TXT[path] = None
            return None
        if b.startswith(b"\xef\xbb\xbf"):
            b = b[3:]
        t = b.decode("utf-8", "replace")
        lines = t.split("\n")
        starts = []
        acc = 0
        for ln in lines:
            starts.append(acc)
            acc += len(ln.encode("utf-8")) + 1
        TXT[path] = (lines, starts)
    v = TXT[path]
    if v is None or line >= len(v[0]):
        return None
    lines, starts = v
    ln = lines[line]
    # UTF-16 -> code points: count surrogate pairs (approximate: astral chars take 2 units)
    cp, units = 0, 0
    while cp < len(ln) and units < col:
        units += 2 if ord(ln[cp]) > 0xFFFF else 1
        cp += 1
    return starts[line] + len(ln[:cp].encode("utf-8"))


# ---------------------------------------------------------------- finder functions
class Funcs:
    def __init__(self, repos):
        self.by_file = defaultdict(list)
        self.qclasses = {}  # (file, name) -> True for v0 qualifying classes
        self.ctor_by_class = {}  # (file, class) -> func
        self.files = set()
        regions = json.load(open(os.path.join(FINDER, "regions.json")))["repos"]
        for repo in repos:
            for f in json.load(open(os.path.join(HOT, "allfuncs", repo + ".allfuncs.json"))):
                f["repo"] = repo
                self.by_file[f["file"]].append(f)
                if f["kind"] == "ctor" and f.get("class"):
                    self.ctor_by_class[(f["file"], f["class"])] = f
            for c in regions[repo]["qualifying_classes"]:
                self.qclasses[(c["file"], c["name"])] = c
        for p, lst in self.by_file.items():
            lst.sort(key=lambda f: f["s"])
            self.files.add(p)
        self.starts = {p: [f["s"] for f in lst] for p, lst in self.by_file.items()}

    def innermost(self, path, off):
        lst = self.by_file.get(path)
        if not lst:
            return None
        i = bisect.bisect_right(self.starts[path], off) - 1
        while i >= 0:
            f = lst[i]
            if f["s"] <= off < f["e"]:
                return f
            i -= 1
        return None


FACT_FILES = {}


def iface_names(repos):
    for repo in repos:
        with open(os.path.join(FINDER, "out", repo + ".jsonl")) as fh:
            FACT_FILES[repo] = json.loads(fh.readline())["files"]


def name_at(repo, fi, off):
    import re
    try:
        b = open(FACT_FILES[repo][fi], "rb").read()
    except Exception:
        return "?"
    if b.startswith(b"\xef\xbb\xbf"):
        b = b[3:]
    t = b[off:off + 300].decode("utf-8", "replace")
    m = re.match(r"\s*(?:/\*.*?\*/\s*|//[^\n]*\n\s*)*(?:export\s+)?(?:declare\s+)?(?:interface|type|class)\s+([A-Za-z0-9_$]+)", t, re.S)
    if m:
        return m.group(1)
    m = re.match(r"\s*([A-Za-z0-9_$]+)", t)
    return (m.group(1) if m else t[:20].replace("\n", " ")) + "?"


def pretty(repo, txt):
    import re
    return re.sub(r"#(\d+):(\d+)", lambda m: name_at(repo, int(m.group(1)), int(m.group(2))), txt or "")


def short_path(p):
    for key in ("/repos/", "/corpus/"):
        if key in p:
            return p.split(key, 1)[1]
    return p


def blocker(f):
    """compact reason string for a non-qualifying function"""
    if f is None:
        return ""
    rs = []
    for r in f["v0_reasons"]:
        if r == "param-type" and f.get("bad_v0"):
            rs.append("param-type: " + f["bad_v0"].replace("param ", ""))
        elif r == "local-type" and f.get("bad_v0"):
            rs.append("local-type: " + f["bad_v0"].replace("local ", ""))
        elif r == "this-class-not-qualifying":
            rs.append("class %s blocked by %s" % (f.get("class"), "/".join(f.get("class_rules", []))))
        else:
            rs.append(r)
    seen, out = set(), []
    for r in rs:
        if r not in seen:
            seen.add(r)
            out.append(r)
    return "; ".join(out)


def main():
    label, repos, outp = sys.argv[1], sys.argv[2].split(","), sys.argv[3]
    profs = sys.argv[4:]
    F = Funcs(repos)
    iface_names(repos)
    frame_cache = {}

    def resolve(cf):
        key = (cf["url"], cf["lineNumber"], cf["columnNumber"], cf["functionName"])
        if key in frame_cache:
            return frame_cache[key]
        name, url = cf["functionName"], cf["url"]
        r = {"cat": None, "func": None, "orig": None, "name": name}
        if name in VM_FRAMES and not url:
            r["cat"] = name
        elif name.startswith("RegExp: ") and not url:
            r["cat"] = "regexp"  # irregexp code
        elif not url and cf["scriptId"] != "0":
            r["cat"] = "eval-js"  # code made by eval / new Function at run time
        elif not url:
            r["cat"] = "native"  # builtins / runtime / API callbacks without script
        elif cf["lineNumber"] == 0 and cf["columnNumber"] == 0 and not name:
            r["cat"] = "module-top-level"  # whole-script / ESM module body (the driver loop)
        elif url.startswith("node:"):
            r["cat"] = "node-internal"
        else:
            sm = smap_for(url)
            if sm is None:
                r["cat"] = "unmapped-js"
            else:
                o = sm.lookup(cf["lineNumber"], cf["columnNumber"])
                if o is None:
                    r["cat"] = "unmapped-js"
                else:
                    path, ol, oc = o
                    r["orig"] = (path, ol + 1, oc + 1)
                    if path not in F.files:
                        r["cat"] = "js-not-analyzed"
                    else:
                        off = byte_offset(path, ol, oc)
                        f = F.innermost(path, off) if off is not None else None
                        # class field initializers / implicit ctor: V8 reports the class start or class body '{'
                        if name and (f is None or (f.get("name") != name and not name.endswith(f.get("name", "")))):
                            c = F.ctor_by_class.get((path, name))
                            if c is not None:
                                f = c
                            elif (path, name) in F.qclasses or any(x.get("class") == name for x in F.by_file[path]):
                                r["cat"] = "class-init"
                                r["class_q"] = (path, name) in F.qclasses
                                r["func"] = None
                                r["name"] = "<%s field init / implicit ctor>" % name
                                frame_cache[key] = r
                                return r
                        if f is None:
                            r["cat"] = "module-top-level"
                        else:
                            r["cat"] = "fn"
                            r["func"] = f
        frame_cache[key] = r
        return r

    tot = Counter()
    per_func_self = Counter()
    per_func_selfb = Counter()
    per_func_incl = Counter()
    func_obj = {}
    cats = Counter()
    breakdown = Counter()
    mism = Counter()
    nsamples = 0
    interval_us = []
    for pf in profs:
        d = json.load(open(pf))
        nodes = {n["id"]: n for n in d["nodes"]}
        parent = {}
        for n in d["nodes"]:
            for c in n.get("children", []):
                parent[c] = n["id"]
        # per node: resolved frame and its stack (leaf->root list of resolved frames)
        stack_cache = {}

        def stack(nid):
            if nid in stack_cache:
                return stack_cache[nid]
            out = []
            x = nid
            while x is not None:
                out.append(resolve(nodes[x]["callFrame"]))
                x = parent.get(x)
            stack_cache[nid] = out
            return out

        samples = d["samples"]
        deltas = d.get("timeDeltas", [])
        if deltas:
            interval_us.append((d["endTime"] - d["startTime"]) / max(1, len(samples)))
        node_summary = {}
        for nid in samples:
            if nid in node_summary:
                key = node_summary[nid]
            else:
                st = stack(nid)
                leaf = st[0]
                # nearest JS frame from the leaf (skips native builtins)
                near = None
                for fr in st:
                    if fr["cat"] in ("fn", "class-init", "js-not-analyzed", "unmapped-js", "node-internal", "module-top-level", "eval-js"):
                        near = fr
                        break
                fn_in_stack = []
                seen = set()
                for fr in st:
                    if fr["cat"] == "fn" and fr["func"]["id"] not in seen:
                        seen.add(fr["func"]["id"])
                        fn_in_stack.append(fr["func"])
                any_v0 = any(f["v0"] for f in fn_in_stack)
                any_v3 = any(f["v3"] for f in fn_in_stack)
                key = (leaf, near, tuple(fn_in_stack), any_v0, any_v3)
                node_summary[nid] = key
            leaf, near, fns, any_v0, any_v3 = key
            nsamples += 1
            lc = leaf["cat"]
            cats[lc if lc != "fn" else ("fn-v0" if leaf["func"]["v0"] else ("fn-v3only" if leaf["func"]["v3"] else "fn-noq"))] += 1
            # self (leaf)
            if lc == "fn":
                f = leaf["func"]
                vn = leaf["name"] or ""
                fnm = f["name"]
                last = vn.replace("get ", "").replace("set ", "").split(".")[-1].lstrip("#")
                if fnm in ("<anon>", "<anon>") or fnm.lstrip("#") == last or (f["kind"] == "ctor" and vn == f.get("class")) or last.rstrip("0123456789") == fnm.lstrip("#"):
                    tot["name_match"] += 1
                elif not vn:
                    tot["name_v8_anon"] += 1
                else:
                    tot["name_mismatch"] += 1
                    mism[(vn, fnm, f["line"], short_path(f["file"]))] += 1
                func_obj[f["id"]] = f
                per_func_self[f["id"]] += 1
                tot["self_fn"] += 1
                if f["v0"]:
                    tot["self_v0"] += 1
                if f["v3"]:
                    tot["self_v3"] += 1
            elif lc == "class-init":
                tot["self_classinit"] += 1
                if leaf.get("class_q"):
                    tot["self_classinit_q"] += 1
            if lc not in VM_FRAMES:
                tot["nonvm"] += 1
            # breakdown of all samples by the nearest JS frame (native/regexp leafs go to their JS caller)
            if near is None:
                bk = leaf["cat"] if leaf["cat"] in VM_FRAMES else "native-no-js-caller"
            elif near["cat"] == "fn":
                bk = "Q-v0" if near["func"]["v0"] else ("Q-v3-only" if near["func"]["v3"] else "not-Q")
            elif near["cat"] == "class-init":
                bk = "class-init-Q" if near.get("class_q") else "class-init-not-Q"
            else:
                bk = near["cat"]
            breakdown[bk] += 1
            # self + builtins: attribute native leaf to nearest JS frame
            if near is not None and near["cat"] == "fn":
                f = near["func"]
                func_obj[f["id"]] = f
                per_func_selfb[f["id"]] += 1
                tot["selfb_fn"] += 1
                if f["v0"]:
                    tot["selfb_v0"] += 1
                if f["v3"]:
                    tot["selfb_v3"] += 1
            if near is not None:
                tot["selfb_js"] += 1
                if near["cat"] in ("fn", "class-init", "module-top-level"):
                    tot["selfb_analyzed_file"] += 1
            # inclusive
            for f in fns:
                per_func_incl[f["id"]] += 1
            if fns:
                tot["incl_any_analyzed"] += 1
            if any_v0:
                tot["incl_v0"] += 1
            if any_v3:
                tot["incl_v3"] += 1
    def pct(a, b):
        return round(100.0 * a / b, 1) if b else None

    res = {
        "label": label,
        "profiles": profs,
        "samples": nsamples,
        "mean_sample_interval_us": round(sum(interval_us) / len(interval_us), 1) if interval_us else None,
        "leaf_categories": dict(cats.most_common()),
        "breakdown_pct": {k: round(100.0 * v / max(1, nsamples), 1) for k, v in breakdown.most_common()},
        "pct_of_all_samples": {
            "self_v0": pct(tot["self_v0"], nsamples),
            "self_v3": pct(tot["self_v3"], nsamples),
            "self_in_analyzed_functions": pct(tot["self_fn"], nsamples),
            "self_plus_builtins_v0": pct(tot["selfb_v0"], nsamples),
            "self_plus_builtins_v3": pct(tot["selfb_v3"], nsamples),
            "self_plus_builtins_in_analyzed_functions": pct(tot["selfb_fn"], nsamples),
            "inclusive_v0": pct(tot["incl_v0"], nsamples),
            "inclusive_v3": pct(tot["incl_v3"], nsamples),
            "inclusive_any_analyzed": pct(tot["incl_any_analyzed"], nsamples),
            "vm_gc_program_idle": pct(nsamples - tot["nonvm"], nsamples),
        },
        "pct_of_analyzed_self_plus_builtins": {
            "v0": pct(tot["selfb_v0"], tot["selfb_fn"]),
            "v3": pct(tot["selfb_v3"], tot["selfb_fn"]),
        },
        "pct_of_analyzed_self": {
            "v0": pct(tot["self_v0"], tot["self_fn"]),
            "v3": pct(tot["self_v3"], tot["self_fn"]),
        },
        "counts": dict(tot),
    }

    def frow(fid, cnt, base):
        f = func_obj[fid]
        return {
            "name": (f.get("class") + "." if f.get("class") else "") + f["name"],
            "kind": f["kind"],
            "at": "%s:%d" % (short_path(f["file"]), f["line"]),
            "pct": pct(cnt, base),
            "samples": cnt,
            "v0": f["v0"], "v3": f["v3"],
            "nested_in_function": f.get("parent") is not None,
            "blocker_v0": "" if f["v0"] else pretty(f["repo"], blocker(f)),
            "v3_reasons": f["v3_reasons"],
        }
    for fid in per_func_incl:
        if fid not in func_obj:
            pass
    res["top_self"] = [frow(fid, c, nsamples) for fid, c in per_func_self.most_common(15)]
    res["top_self_plus_builtins"] = [frow(fid, c, nsamples) for fid, c in per_func_selfb.most_common(15)]
    # inclusive needs func objects for all stack functions
    allf = {}
    for fs in F.by_file.values():
        for f in fs:
            allf[f["id"]] = f
    func_obj.update({k: allf[k] for k in per_func_incl if k in allf})
    res["top_inclusive"] = [frow(fid, c, nsamples) for fid, c in per_func_incl.most_common(15)]
    res["top_inclusive_v0"] = [frow(fid, c, nsamples) for fid, c in per_func_incl.most_common() if fid in allf and allf[fid]["v0"]][:15]
    res["top_self_v0"] = [frow(fid, c, nsamples) for fid, c in per_func_selfb.most_common() if func_obj[fid]["v0"]][:15]
    # blocker histogram over self+builtins time of non-qualifying functions
    bh = Counter()
    bh_first = Counter()
    for fid, c in per_func_selfb.items():
        f = func_obj[fid]
        if f["v0"]:
            continue
        for r in set(f["v0_reasons"]):
            bh[r] += c
        if f["v0_reasons"]:
            bh_first[f["v0_reasons"][0]] += c
    res["blocker_time_pct_any"] = {k: pct(v, nsamples) for k, v in bh.most_common()}
    res["blocker_time_pct_first"] = {k: pct(v, nsamples) for k, v in bh_first.most_common()}
    # bad param/local type kinds weighted by time
    bt = Counter()
    for fid, c in per_func_selfb.items():
        f = func_obj[fid]
        if f["v0"] or not f.get("bad_v0"):
            continue
        k = f["bad_v0"].split(" ", 1)[1] if " " in f["bad_v0"] else f["bad_v0"]
        bt[pretty(f["repo"], k)] += c
    res["bad_type_time_pct"] = {k: pct(v, nsamples) for k, v in bt.most_common(12)}
    # static shares for comparison (root files only)
    allroot = [f for f in allf.values() if f["root"]]
    res["static_fn_share"] = {"v0": pct(sum(f["v0"] for f in allroot), len(allroot)), "v3": pct(sum(f["v3"] for f in allroot), len(allroot))}
    # distinct functions that ever got a sample and their qualification
    hotset = set(per_func_selfb)
    res["distinct_sampled_functions"] = len(hotset)
    res["distinct_sampled_v0"] = sum(1 for fid in hotset if func_obj[fid]["v0"])
    # how concentrated is the time: share in top-N functions
    tot_fn = sum(per_func_selfb.values())
    acc, n50 = 0, 0
    for fid, c in per_func_selfb.most_common():
        acc += c
        n50 += 1
        if acc * 2 >= tot_fn:
            break
    res["functions_for_50pct_of_analyzed_time"] = n50
    res["name_check"] = {"match": tot["name_match"], "v8_anonymous": tot["name_v8_anon"], "mismatch": tot["name_mismatch"],
                          "top_mismatch": [list(k) + [v] for k, v in mism.most_common(8)]}
    res["per_function"] = [
        {"id": fid, "file": func_obj[fid]["file"], "s": func_obj[fid]["s"], "e": func_obj[fid]["e"], "name": func_obj[fid]["name"],
         "kind": func_obj[fid]["kind"], "class": func_obj[fid].get("class"), "line": func_obj[fid]["line"],
         "v0": func_obj[fid]["v0"], "v3": func_obj[fid]["v3"], "repo": func_obj[fid]["repo"],
         "selfb": per_func_selfb.get(fid, 0), "self": per_func_self.get(fid, 0), "incl": per_func_incl.get(fid, 0)}
        for fid in set(per_func_selfb) | set(per_func_self) | set(per_func_incl) if fid in func_obj]
    res["per_function"].sort(key=lambda r: -r["selfb"])
    json.dump(res, open(outp, "w"), indent=1)
    print(json.dumps({k: v for k, v in res.items() if not k.startswith("top")}, indent=1))
    for key in ("top_self_plus_builtins",):
        print("\n" + key)
        for r in res[key][:12]:
            print("  %5.1f%%  %-4s %-4s %-45s %-50s %s" % (r["pct"], "Q0" if r["v0"] else ("Q3" if r["v3"] else "--"), r["kind"][:4], r["name"][:45], r["at"][:50], r["blocker_v0"][:120]))


if __name__ == "__main__":
    main()
