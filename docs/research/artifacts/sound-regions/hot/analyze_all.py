#!/usr/bin/env python3
"""Sound-region finder v0: apply rule set r1..r7 to the facts written by region_facts (tsrs) and report
how much real code qualifies. Usage: python3 -I analyze.py <repo>... (reads out/<repo>.jsonl, projects/<repo>/roots.txt)
Writes out/<repo>.summary.json and (all repos) regions.json."""
import json
import os
import sys
from collections import Counter, defaultdict

HERE = "$SR/finder"
HOTOUT = "$SR/hot/allfuncs"
OUT = os.path.join(HERE, "out")
PRIM = {"number", "string", "boolean", "bigint"}


def load(repo):
    lines = [json.loads(l) for l in open(os.path.join(OUT, repo + ".jsonl"))]
    files = lines[0]["files"]
    recs = lines[1:-1]
    stats = lines[-1]["stats"]
    roots = set(l.strip() for l in open(os.path.join(HERE, "projects", repo, "roots.txt")) if l.strip())
    return files, recs, stats, roots


class Lines:
    def __init__(self):
        self.cache = {}

    def line(self, path, off):
        if path not in self.cache:
            b = open(path, "rb").read()
            if b.startswith(b"\xef\xbb\xbf"):
                b = b[3:]
            starts = [0]
            for i, ch in enumerate(b):
                if ch == 10:
                    starts.append(i + 1)
            self.cache[path] = starts
        st = self.cache[path]
        lo, hi = 0, len(st)
        while lo + 1 < hi:
            mid = (lo + hi) // 2
            if st[mid] <= off:
                lo = mid
            else:
                hi = mid
        return lo + 1


LN = Lines()


def walk_struct_ids(t, out):
    """project structural type ids (iface / type literal / object literal) inside a target descriptor"""
    if not isinstance(t, dict):
        return
    c = t.get("c")
    if c in ("iface", "tlit", "objlit") and "id" in t:
        out.add(t["id"])
    for k in ("i", "el"):
        if k in t:
            walk_struct_ids(t[k], out)
    for m in t.get("m", []) or []:
        walk_struct_ids(m, out)


def analyze(repo):
    files, recs, stats, roots = load(repo)
    classes = {}
    funcs = {}
    import re
    privname = re.compile(r"^\x7f?#[0-9]+@(#.*)$")
    file_of = {}
    r7_files = set()
    casts, flows, ifl, members, r3s, calls, vrefs = [], [], [], [], [], [], []
    for r in recs:
        fi = r["fi"]
        path = r["file"]
        file_of[fi] = path
        if r["r7"]:
            r7_files.add(fi)
        fids = [f["id"] for f in r["funcs"]]
        for c in r["classes"]:
            c["fi"] = fi
            for f in c["fields"]:
                m = privname.match(f["n"])
                if m:
                    f["n"] = m.group(1)  # tsrs mangles #private symbol names as "#<id>@#name"
            classes[c["id"]] = c
        for f in r["funcs"]:
            f["fi"] = fi
            f["pid"] = fids[f["parent"]] if f["parent"] is not None else None
            funcs[f["id"]] = f

        def fid(i):
            return fids[i] if i is not None else None

        for x in r["casts"]:
            x["fi"] = fi
            x["fid"] = fid(x["f"])
            casts.append(x)
        for x in r["flows"]:
            x["fi"] = fi
            x["fid"] = fid(x["f"])
            flows.append(x)
        for x in r["ifl"]:
            x["fi"] = fi
            ifl.append(x)
        for x in r["members"]:
            x["fi"] = fi
            members.append(x)
        for x in r["r3"]:
            x["fi"] = fi
            r3s.append(x)
        for x in r["calls"]:
            x["fi"] = fi
            x["fid"] = fid(x["f"])
            calls.append(x)
        for v in r["vrefs"]:
            vrefs.append({"fid": fid(v[0]), "t": v[1], "s": v[2], "fi": fi})
    root_fi = {fi for fi, p in file_of.items() if p in roots}

    # ---------------- hierarchy
    parent = {}
    for cid, c in classes.items():
        e = c.get("ext")
        if e and "id" in e and e["id"] in classes:
            parent[cid] = e["id"]
    children = defaultdict(list)
    for k, v in parent.items():
        children[v].append(k)

    def ancestors(cid):
        out = []
        seen = set()
        while cid in parent and parent[cid] not in seen:
            cid = parent[cid]
            seen.add(cid)
            out.append(cid)
        return out

    def descendants(cid):
        out, stack = [], list(children.get(cid, []))
        while stack:
            x = stack.pop()
            if x in out:
                continue
            out.append(x)
            stack.extend(children.get(x, []))
        return out

    anc = {cid: set(ancestors(cid)) for cid in classes}

    reasons = defaultdict(list)  # cid -> [(rule, sub, fi, s)]

    def add(cid, rule, sub, fi, s, prop=False, guard=None):
        if cid not in classes:
            return
        reasons[cid].append((rule, sub, fi, s, guard))
        if prop:
            for d in descendants(cid):
                reasons[d].append((rule, sub + "(via base)", fi, s, guard))

    # r1: casts and any-flows
    for x in casts:
        tgt = set(x["td"]) | set(x["tn"])
        src = set(x["sd"]) | set(x["sn"])
        # guardable: `!` (null check), upcast (no-op), class-to-class downcast (brand check)
        if x["sc"] in ("any", "unknown", "error"):
            x["up"] = False  # any is assignable to everything: a cast from any is unchecked
        # guardable only when the target is a class type: `!`, class<->class casts, upcasts into a class
        x["guard"] = x["k"] == "nn" or bool(x["sd"] and x["td"]) or bool(x["up"] and x["td"])
        if x["k"] == "nn":
            for c in tgt:
                add(c, "r1", "nonnull-!", x["fi"], x["s"], guard="cast")
        else:
            sub = "as-up" if x["up"] else ("as-down" if x["guard"] else "as-unchecked")
            for c in tgt:
                add(c, "r1", sub + "(to C)", x["fi"], x["s"], guard="cast" if x["guard"] else None)
            for c in src - tgt:
                add(c, "r1", sub + "(from C)", x["fi"], x["s"], guard="cast" if x["guard"] else None)
    for x in flows:
        if x["k"] == "any":
            for c in set(x["td"]) | set(x["tn"]):
                add(c, "r1", "%s-flows-in" % x["sa"], x["fi"], x["s"])
        elif x["k"] == "fake":
            for c in x["td"]:
                add(c, "r1", "structural-fake(%s)" % x["sdesc"].get("c"), x["fi"], x["s"])
    # r2
    for m in members:
        if m.get("el") and not m.get("ck") and m["d"]:
            for c in m["d"]:
                add(c, "r2", "dynamic-key", m["fi"], m["s"], prop=True)
    # r3
    for x in r3s:
        for c in x["inst"]:
            add(c, "r3", x["k"], x["fi"], x["s"], prop=True)
        for c in x.get("ctor", []):
            add(c, "r3", x["k"] + "(ctor/proto)", x["fi"], x["s"], prop=True)
    for cid, c in classes.items():
        if c["dec"]:
            add(cid, "r3", "class-decorator", c["fi"], c["s"], prop=True)
        if c["mdec"]:
            add(cid, "r3", "member-decorator", c["fi"], c["s"], prop=True)

    # r4 (strict and lenient)
    def before_leak(c, lenient):
        aa = c["aa"]
        for k, s, cnt in c.get("ev", []):
            if lenient and k in ("method-call", "accessor"):
                continue
            return set(aa[:cnt]), k
        return set(aa), None

    def any_leak(c, lenient):
        for k, s, cnt in c.get("ev", []):
            if lenient and k in ("method-call", "accessor"):
                continue
            return True
        return False

    r4_reasons = {False: defaultdict(list), True: defaultdict(list)}
    for lenient in (False, True):
        for cid, c in classes.items():
            bl, lk = before_leak(c, lenient)
            own_fields = c["fields"]
            for f in own_fields:
                dk = f.get("dk", [])
                if "pp" in dk or f["n"] in bl:
                    continue
                if f.get("abstract") or (f.get("declare") and f["n"] not in c["aa"]):
                    continue  # abstract: defined by subclasses; declare: type-only (no runtime field)
                if "merged" in dk and len(dk) == 1:
                    continue  # handled under r6
                if dk and all(d == "js-late" for d in dk):
                    sub = "late-js-field"
                elif f.get("opt"):
                    if lenient:
                        continue  # type-safe view: an optional field may stay undefined
                    sub = "optional-unassigned"
                elif lenient and not f.get("def") and (f["t"].get("c") in ("nullable", "any", "unknown") or f["t"].get("nul")):
                    continue  # type-safe view: the type admits undefined (approximation: null-only types also pass)
                elif f["n"] in c["aa"]:
                    sub = "assigned-after-leak(%s)" % lk
                else:
                    sub = "never-assigned-in-ctor" + ("(!)" if f.get("def") else "") + ("(declare)" if f.get("declare") else "")
                r4_reasons[lenient][cid].append((sub, f["n"]))
            if own_fields and any(any_leak(classes[a], lenient) for a in anc[cid]):
                r4_reasons[lenient][cid].append(("base-ctor-leaks-this", ""))

    # r6
    for cid, c in classes.items():
        e = c.get("ext")
        if e and "foreign" in e:
            add(cid, "r6", "foreign-base(%s)" % e["foreign"], c["fi"], c["s"])
        elif e and "id" in e and e["id"] not in classes:
            add(cid, "r6", "base-outside-walked-files", c["fi"], c["s"])
        if c["index"]:
            add(cid, "r6", "index-signature", c["fi"], c["s"])
        if any("merged" in f.get("dk", []) for f in c["fields"]):
            add(cid, "r6", "merged-interface-decl", c["fi"], c["s"])
    inst_members = {cid: {m["n"]: m["k"] for m in c["members"] if not m["st"]} for cid, c in classes.items()}
    for cid, c in classes.items():
        mine = inst_members[cid]
        for d in descendants(cid):
            for n, k in inst_members[d].items():
                if n in mine and (mine[n] in ("get", "set", "acc") or k in ("get", "set", "acc")):
                    add(cid, "r6", "accessor-overridden(%s)" % n, classes[d]["fi"], classes[d]["s"])
    # r7
    for cid, c in classes.items():
        if c["fi"] in r7_files:
            add(cid, "r7", "eval/with/Function in module", c["fi"], c["s"])

    # r5: class escapes
    def classify(t, cid):
        c = t.get("c")
        if c in ("any", "error"):
            return "throw" if t.get("throw") else "any"
        if c == "unknown":
            return "unknown"
        if c == "none":
            return "ok-inferred"
        if c == "object":
            return "object"
        if c == "emptyobj":
            return "{}"
        if c == "nullable":
            return classify(t.get("i", {}), cid)
        if c == "array":
            return classify(t["el"], cid) if "el" in t else "other"
        if c in ("union", "inter", "tuple"):
            ks = [classify(m, cid) for m in t.get("m", t.get("ta", []))]
            if any(k.startswith("up") for k in ks):
                return "up"
            ifs = [k for k in ks if k.startswith("iface:")]
            if ifs:
                return ifs[0]
            nonprim = [k for k in ks if k != "prim"]
            return (nonprim or ks or ["other"])[0]
        if c == "inst":
            if "id" in t:
                return "up" if (t["id"] == cid or t["id"] in anc.get(cid, ())) else "class-mismatch"
            return "foreign:" + t.get("fn", "?")
        if c in ("iface", "tlit", "objlit"):
            if "id" in t:
                return "iface:" + t["id"]
            return "foreign:" + t.get("fn", "?")
        if c == "tp":
            return classify(t["i"], cid) if "i" in t else "generic-T"
        if c in PRIM:
            return "prim"
        if c in ("void", "undefined", "never"):
            return "ok-inferred"  # value is discarded
        if c == "other":
            return "generic-T"  # unclassified generic type (indexed access / conditional / substitution)
        return "other:" + str(c)

    def tps(t, out):
        if not isinstance(t, dict):
            return
        if t.get("c") == "tp":
            out.append(t)
            return
        for k in ("i", "el"):
            if k in t:
                tps(t[k], out)
        for k in ("m", "ta"):
            for m in t.get(k, []) or []:
                tps(m, out)

    def gt_kind(gt, cid, direct):
        """C flows into a project generic parameter: the generic body sees it as T (its constraint, or opaque)."""
        if direct:
            # the C value itself fills the declared slot (T, T | undefined, C<T>, Base<T>, I<T> ...)
            return classify(gt, cid)
        found = []
        tps(gt, found)
        ks = []
        for tp in found:
            if "i" in tp:
                k = classify(tp["i"], cid)
                if k == "prim":
                    continue
                ks.append("generic-T" if k in ("{}", "unknown", "object") else k)
            else:
                ks.append("generic-T")
        bad = [k for k in ks if k not in ("up", "ok-inferred")]
        return bad[0] if bad else "up"

    esc_static = defaultdict(list)  # cid -> [(kind, fi, s)]
    esc_kinds = Counter()
    esc_iface = defaultdict(set)  # cid -> iface ids
    iface_src = defaultdict(set)  # iface id -> {"class:C" | "other" | "iface:J"}
    for x in flows:
        if x["k"] == "any":
            ids = set()
            walk_struct_ids(x.get("t", {}), ids)
            for i in ids:
                iface_src[i].add("other")
            continue
        if x["k"] == "fake":
            continue
        tgt = set(x["td"]) | set(x["tn"])
        for cid in set(x["sd"]) | set(x["sn"]):
            if cid not in classes:
                continue
            k = "up" if cid in tgt else classify(x["t"], cid)
            if x.get("lit") and cid not in x["sd"] and k not in ("any", "unknown", "object", "{}", "throw"):
                k = "up"  # literal container: its elements/properties are checked as their own flows
            if k in ("up", "ok-inferred") and "gt" in x:
                k = gt_kind(x["gt"], cid, cid in x["sd"])
            if k.startswith("iface:"):
                esc_iface[cid].add(k[6:])
                iface_src[k[6:]].add("class:" + cid)
            elif k in ("up", "ok-inferred"):
                pass
            else:
                esc_static[cid].append((k, x["fi"], x["s"]))
                esc_kinds[k] += 1
                if k == "class-mismatch":
                    # the receiving class gets a value that is not one of its instances
                    for tc in x["td"]:
                        if tc != cid and tc not in anc.get(cid, ()):
                            add(tc, "r1", "structural-fake(class)", x["fi"], x["s"])
    for x in ifl:
        ids = set()
        walk_struct_ids(x["t"], ids)
        sd = x.get("sdesc", {})
        src = "other"
        if sd.get("c") in ("iface", "tlit") and "id" in sd:
            src = "iface:" + sd["id"]
        for i in ids:
            if src != "iface:" + i:
                iface_src[i].add(src)
    for cid, lst in esc_static.items():
        for k, fi, s in lst:
            add(cid, "r5", k.split(":")[0] if k.startswith("foreign:") else k, fi, s, guard="gen" if k == "generic-T" else None)

    # r5 reachability: a class stored in a field of an escaped class is reachable by foreign code too
    def type_ids(t, out):
        if not isinstance(t, dict):
            return
        if t.get("c") == "inst" and "id" in t:
            out.add(t["id"])
        for k in ("i", "el"):
            if k in t:
                type_ids(t[k], out)
        for k in ("m", "ta"):
            for m in t.get(k, []) or []:
                type_ids(m, out)
    field_ids = {}
    for cid, c in classes.items():
        ids = set()
        for f in c["fields"]:
            type_ids(f["t"], ids)
        ids.discard(cid)
        field_ids[cid] = ids
    escaped = {cid for cid, lst in esc_static.items() if any(k != "generic-T" for k, _, _ in lst)}
    work = list(escaped)
    reach_from = {}
    while work:
        h = work.pop()
        for c in field_ids.get(h, ()):
            if c not in escaped and c in classes:
                escaped.add(c)
                reach_from[c] = h
                work.append(c)
    for c, h in reach_from.items():
        add(c, "r5", "reachable-from-escaped(%s)" % classes[h]["name"], classes[h]["fi"], classes[h]["s"])

    # ---------------- fixpoint over classes
    def static_rules(cid, lenient_r4, relax=()):
        rs = {r for r, _, _, _, g in reasons[cid] if not (g and g in relax)}
        if r4_reasons[lenient_r4][cid]:
            rs.add("r4")
        return rs

    def fixpoint(lenient_r4=False, open_world=False, relax=()):
        Q = {cid for cid in classes if not static_rules(cid, lenient_r4, relax)}
        if open_world:
            Q = {cid for cid in Q if not classes[cid]["exp"]}
        dyn = defaultdict(set)
        while True:
            closed = {}
            # greatest fixpoint for interface closedness
            ifs = set(iface_src)
            cl = {i: True for i in ifs}
            changed = True
            while changed:
                changed = False
                for i in ifs:
                    if not cl[i]:
                        continue
                    for s in iface_src[i]:
                        ok = (s.startswith("class:") and s[6:] in Q) or (s.startswith("iface:") and cl.get(s[6:], True))
                        if not ok:
                            cl[i] = False
                            changed = True
                            break
            closed = cl
            removed = set()
            for cid in Q:
                p = parent.get(cid)
                if p is not None and p not in Q:
                    removed.add(cid)
                    dyn[cid].add("base")
                bad = [i for i in esc_iface.get(cid, ()) if not closed.get(i, True)]
                if bad:
                    removed.add(cid)
                    dyn[cid].add("r5")
            if not removed:
                return Q, dyn, closed
            Q -= removed

    Q, dyn, closed = fixpoint(False)
    Q_len, _, _ = fixpoint(True)
    Q_open, _, _ = fixpoint(False, open_world=True)
    Q_g, _, _ = fixpoint(False, relax=("cast",))
    Q_g4, _, _ = fixpoint(True, relax=("cast",))
    Q_g4t, _, _ = fixpoint(True, relax=("cast", "gen"))

    def all_rules(cid):
        rs = static_rules(cid, False) | dyn.get(cid, set())
        if cid not in Q:
            # r5 via shared interfaces for classes removed statically too (for reporting)
            if any(not closed.get(i, True) for i in esc_iface.get(cid, ())):
                rs.add("r5")
            p = parent.get(cid)
            if p is not None and p not in Q:
                rs.add("base")
        return rs

    rc = [cid for cid in classes if classes[cid]["fi"] in root_fi]
    rule_cnt = Counter()
    sole_cnt = Counter()
    sub_cnt = Counter()
    for cid in rc:
        if cid in Q:
            continue
        rs = all_rules(cid)
        for r in rs:
            rule_cnt[r] += 1
        if len(rs) == 1:
            sole_cnt[next(iter(rs))] += 1
        subs = set()
        for r, sub, _, _, _ in reasons[cid]:
            subs.add(r + ":" + sub.split("(")[0] if not sub.startswith("foreign-base") else r + ":foreign-base")
        for sub, _ in r4_reasons[False][cid]:
            subs.add("r4:" + sub.split("(")[0])
        for s in subs:
            sub_cnt[s] += 1

    # leave-one-out: how many root classes qualify if one rule is switched off
    def fix_without(rule):
        def sr(cid):
            rs = static_rules(cid, False)
            rs.discard(rule)
            return rs
        Q2 = {cid for cid in classes if not sr(cid)}
        while True:
            ifs = set(iface_src)
            cl = {i: True for i in ifs}
            changed = True
            while changed:
                changed = False
                for i in ifs:
                    if not cl[i]:
                        continue
                    for s in iface_src[i]:
                        ok = (s.startswith("class:") and s[6:] in Q2) or (s.startswith("iface:") and cl.get(s[6:], True))
                        if not ok:
                            cl[i] = False
                            changed = True
                            break
            removed = set()
            for cid in Q2:
                p = parent.get(cid)
                if rule != "base" and p is not None and p not in Q2:
                    removed.add(cid)
                if rule != "r5" and any(not cl.get(i, True) for i in esc_iface.get(cid, ())):
                    removed.add(cid)
            if not removed:
                return Q2
            Q2 -= removed

    loo = {}
    for r in ("r1", "r2", "r3", "r4", "r5", "r6", "r7", "base"):
        q2 = fix_without(r)
        loo[r] = sum(1 for cid in rc if cid in q2)

    # ---------------- field representation
    def field_kind(t):
        c = t.get("c")
        if c in ("number", "boolean", "string"):
            return c
        if c == "inst" and t.get("id") in Q:
            return "qclass"
        if c == "nullable":
            i = t.get("i", {})
            if i.get("c") in ("number", "boolean", "string"):
                return "nullable-prim"
            if i.get("c") == "inst" and i.get("id") in Q:
                return "nullable-qclass"
        if c == "array":
            return "array"
        return "other"

    fk = Counter()
    for cid in rc:
        if cid in Q:
            for f in classes[cid]["fields"]:
                fk[field_kind(f["t"])] += 1

    # ---------------- functions
    casts_by_f = defaultdict(list)
    for x in casts:
        if x["fid"]:
            casts_by_f[x["fid"]].append(x)

    def func_eval(Qv, guard_casts, closed_ok=False, tp_ok=False):
        def tier_of(t):
            """0 strict (prim | Q class), 1 nullable of those, 2 array of those; None = fails"""
            if t is None:
                return 0
            c = t.get("c")
            if c in PRIM:
                return 0
            if c == "inst" and t.get("id") in Qv:
                return 0
            if closed_ok and c in ("iface", "tlit") and t.get("id") in iface_src and closed.get(t["id"], False):
                return 0  # what-if: interface whose every source is a qualifying class (dispatch over a closed class set)
            if tp_ok and c == "tp" and "i" not in t:
                return 0  # what-if: unconstrained generic T (value is opaque; no layout facts used)
            if c == "nullable":
                ti = tier_of(t.get("i", {}))
                return 1 if ti == 0 else None
            if c == "array" and "el" in t:
                te = tier_of(t["el"])
                return 2 if te is not None else None
            return None

        fs = {}
        for fid_, f in funcs.items():
            rs = []
            tiers = [0]
            for p in f["params"]:
                t = tier_of(p)
                if t is None:
                    rs.append("param-type")
                    break
                tiers.append(t)
            if f["cls"] and not f["st"] and f["cls"] not in Qv:
                rs.append("this-class-not-qualifying")
            if f.get("this") and tier_of(f["this"]) is None:
                rs.append("this-type")
            for l in f["locals"]:
                t = tier_of(l)
                if t is None:
                    rs.append("local-type")
                    break
                tiers.append(t)
            if guard_casts:
                if any(x["k"] == "as" and not x["guard"] for x in casts_by_f.get(fid_, [])):
                    rs.append("assertion(as)")
            else:
                if f["asrt"] or f["up"]:
                    rs.append("assertion(as)")
                if f["nn"]:
                    rs.append("nonnull(!)")
            if f["anyflow"]:
                rs.append("any-flow")
            if f["r2"]:
                rs.append("r2-dynamic-key")
            if f["fi"] in r7_files:
                rs.append("r7")
            fs[fid_] = (rs, max(tiers))
        return fs

    fstat = func_eval(Q, False)
    qual = {fid_ for fid_, (rs, _) in fstat.items() if not rs}
    q_t0 = {fid_ for fid_ in qual if fstat[fid_][1] == 0}
    q_t1 = {fid_ for fid_ in qual if fstat[fid_][1] <= 1}
    fstat_g = func_eval(Q_g, True)
    qual_g = {fid_ for fid_, (rs, _) in fstat_g.items() if not rs}
    fstat_g4 = func_eval(Q_g4, True)
    qual_g4 = {fid_ for fid_, (rs, _) in fstat_g4.items() if not rs}
    fstat_g4t = func_eval(Q_g4t, True)
    fstat_w1 = func_eval(Q, False, closed_ok=True)
    fstat_w2 = func_eval(Q, False, closed_ok=True, tp_ok=True)
    fstat_w3 = func_eval(Q_g4t, True, closed_ok=True, tp_ok=True)
    qual_g4t = {fid_ for fid_, (rs, _) in fstat_g4t.items() if not rs}

    # call sites / boundaries
    incoming = defaultdict(list)
    for x in calls:
        if x["t"]:
            incoming[x["t"]].append(x)
    # virtual dispatch: a call resolved to A.m may land on override C.m
    by_cls_name = {}
    for fid_, f in funcs.items():
        if f["cls"] and f["k"] in ("method", "get", "set"):
            by_cls_name[(f["cls"], f["n"])] = fid_
    # calls through interface method signatures: attribute to same-name methods of classes that implement
    # the interface (declared `implements`) or flow into it (r5 interface escapes)
    iface_calls = defaultdict(list)
    for x in calls:
        if x.get("ti") and x.get("m"):
            iface_calls[(x["ti"], x["m"])].append(x)
    cls_ifaces = defaultdict(set)
    for cid, c in classes.items():
        for i in c.get("impl", []):
            cls_ifaces[cid].add(i)
        for i in esc_iface.get(cid, ()):
            cls_ifaces[cid].add(i)
    vref_in = defaultdict(list)
    for v in vrefs:
        vref_in[v["t"]].append(v)
    def boundaries(qset):
      fb = {}
      for fid_ in qset:
        f = funcs[fid_]
        sites = list(incoming.get(fid_, []))
        if f["cls"] and f["k"] in ("method", "get", "set"):
            for a in anc.get(f["cls"], ()):
                base_m = by_cls_name.get((a, f["n"]))
                if base_m:
                    sites += incoming.get(base_m, [])
            for c in [f["cls"]] + list(anc.get(f["cls"], ())):
                for i in cls_ifaces.get(c, ()):
                    sites += iface_calls.get((i, f["n"]), [])
        internal = sum(1 for x in sites if x["fid"] in qset)
        bound = len(sites) - internal
        vr = len(vref_in.get(fid_, []))
        esc = 1 if f["k"] in ("arrow", "fnexpr") and f["ctx"] not in ("iife", "var") else 0
        fb[fid_] = {"internal": internal, "boundary_calls": bound, "value_refs": vr, "escaping_closure": esc}
      return fb
    fboundary = boundaries(qual)
    fboundary_v3 = boundaries(qual_g4t)

    rf = [fid_ for fid_ in funcs if funcs[fid_]["fi"] in root_fi]
    rq = [fid_ for fid_ in rf if fid_ in qual]
    freason = Counter()
    fsole = Counter()
    for fid_ in rf:
        rs = fstat[fid_][0]
        for r in set(rs):
            freason[r] += 1
        if len(set(rs)) == 1:
            fsole[rs[0]] += 1
    kind_cnt = Counter(funcs[f]["k"] for f in rf)
    kind_q = Counter(funcs[f]["k"] for f in rq)
    nontrivial_q = [f for f in rq if funcs[f]["params"] or funcs[f]["locals"]]
    b_calls = sum(fboundary[f]["boundary_calls"] for f in rq)
    b_vref = sum(fboundary[f]["value_refs"] for f in rq)
    b_esc = sum(fboundary[f]["escaping_closure"] for f in rq)
    internal = sum(fboundary[f]["internal"] for f in rq)
    closed_regions = sum(1 for f in rq if fboundary[f]["boundary_calls"] + fboundary[f]["value_refs"] + fboundary[f]["escaping_closure"] == 0)
    exported_q = sum(1 for f in rq if funcs[f]["exp"])

    # ---------------- member accesses (root files)
    rm = [m for m in members if m["fi"] in root_fi]
    dyn_m = [m for m in rm if m.get("rc") not in ("ns", "ctor")]
    cls_m = [m for m in dyn_m if len(m["d"]) >= 1]
    q_m = [m for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q]
    q_m_fn = [m for m in q_m if m.get("f") is not None]

    def loc(fi, s):
        p = file_of[fi]
        return "%s:%d" % (p, LN.line(p, s))

    # examples
    def cls_ex(cid):
        c = classes[cid]
        d = {"name": c["name"], "at": loc(c["fi"], c["s"]), "fields": len(c["fields"]),
             "field_kinds": dict(Counter(field_kind(f["t"]) for f in c["fields"]))}
        if cid not in Q:
            rs = sorted(all_rules(cid))
            d["rules"] = rs
            ev = []
            for r, sub, fi, s, _ in reasons[cid][:4]:
                ev.append("%s %s @ %s" % (r, sub, loc(fi, s)))
            for sub, fname in r4_reasons[False][cid][:2]:
                ev.append("r4 %s field=%s" % (sub, fname))
            if "base" in rs:
                ev.append("base %s" % classes[parent[cid]]["name"])
            if "r5" in rs and esc_iface.get(cid):
                bad = [i for i in esc_iface[cid] if not closed.get(i, True)]
                ev.append("r5 escapes to shared structural type(s) %s" % bad[:2])
            d["evidence"] = ev
        return d

    q_root = [cid for cid in rc if cid in Q]
    q_examples = [cls_ex(c) for c in sorted(q_root, key=lambda c: -len(classes[c]["fields"]))[:3]]
    blocked = [cid for cid in rc if cid not in Q]
    b_examples = []
    seen_rules = set()
    picked = []
    for cid in sorted(blocked, key=lambda c: -len(classes[c]["fields"])):
        rs = frozenset(all_rules(cid))
        key = min(rs) if rs else ""
        if key in seen_rules:
            continue
        seen_rules.add(key)
        picked.append(cid)
        if len(picked) == 3:
            break
    for cid in sorted(blocked, key=lambda c: -len(classes[c]["fields"])):
        if len(picked) >= 3:
            break
        if cid not in picked:
            picked.append(cid)
    b_examples = [cls_ex(c) for c in picked]

    fq_examples = []
    for f in sorted(nontrivial_q, key=lambda f: -(len(funcs[f]["params"]) + len(funcs[f]["locals"])))[:3]:
        ff = funcs[f]
        fq_examples.append({"name": ff["n"], "kind": ff["k"], "at": loc(ff["fi"], ff["s"]), "tier": fstat[f][1], **fboundary[f]})

    # share of function-body bytes covered by qualifying functions (union of intervals per file)
    def union_len(iv):
        iv = sorted(iv)
        tot, cs, ce = 0, None, None
        for s, e in iv:
            if cs is None or s > ce:
                if cs is not None:
                    tot += ce - cs
                cs, ce = s, e
            else:
                ce = max(ce, e)
        if cs is not None:
            tot += ce - cs
        return tot
    by_file_all, by_file_q, by_file_qg = defaultdict(list), defaultdict(list), defaultdict(list)
    for f in rf:
        ff = funcs[f]
        by_file_all[ff["fi"]].append((ff["bs"], ff["be"]))
        if f in qual:
            by_file_q[ff["fi"]].append((ff["bs"], ff["be"]))
        if f in qual_g4t:
            by_file_qg[ff["fi"]].append((ff["bs"], ff["be"]))
    body_all = sum(union_len(v) for v in by_file_all.values())
    body_q = sum(union_len(v) for v in by_file_q.values())
    body_qg = sum(union_len(v) for v in by_file_qg.values())
    # member accesses that are inside a qualifying function AND on a qualifying receiver (no guard needed)
    fid_by_fi_idx = {}
    for r in recs:
        for i, f in enumerate(r["funcs"]):
            fid_by_fi_idx[(r["fi"], i)] = f["id"]
    sound_m = sum(1 for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q and m.get("f") is not None and fid_by_fi_idx.get((m["fi"], m["f"])) in qual)
    sound_m_g = sum(1 for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q_g4t and m.get("f") is not None and fid_by_fi_idx.get((m["fi"], m["f"])) in qual_g4t)

    # per-module coverage (K-A5 asks for >= 50% of one real typed module); module = root file with
    # >= 5 functions and >= 2 KB of function-body code
    mods = []
    for fi in root_fi:
        allb = union_len(by_file_all.get(fi, []))
        nf = len(by_file_all.get(fi, []))
        if nf < 5 or allb < 2048:
            continue
        qb = union_len(by_file_q.get(fi, []))
        qb3 = union_len(by_file_qg.get(fi, []))
        mods.append((file_of[fi], nf, allb, qb, qb3))
    mods_ge50 = [m for m in mods if m[3] * 2 >= m[2]]
    mods_ge50_v3 = [m for m in mods if m[4] * 2 >= m[2]]
    top_mods = sorted(mods_ge50_v3, key=lambda m: -m[2])[:3]

    # code size (root files, bytes)
    root_bytes = sum(os.path.getsize(file_of[fi]) for fi in root_fi)

    S = {
        "repo": repo,
        "root_files": len(root_fi),
        "walked_files": len(file_of),
        "root_bytes": root_bytes,
        "diagnostics": stats["diagnostics"],
        "diag_codes": stats["diag_codes"],
        "classes_total": len(rc),
        "classes_qualifying": len(q_root),
        "classes_qualifying_pct": pct(len(q_root), len(rc)),
        "classes_qualifying_if_r4_lenient": sum(1 for c in rc if c in Q_len),
        "classes_qualifying_guarded_casts": sum(1 for c in rc if c in Q_g),
        "classes_qualifying_guarded_casts_r4_lenient": sum(1 for c in rc if c in Q_g4),
        "classes_qualifying_v3_plus_generic_T_ok": sum(1 for c in rc if c in Q_g4t),
        "classes_qualifying_open_world": sum(1 for c in rc if c in Q_open),
        "classes_exported_qualifying": sum(1 for c in q_root if classes[c]["exp"]),
        "class_rule_counts": dict(rule_cnt.most_common()),
        "class_sole_rule_counts": dict(sole_cnt.most_common()),
        "class_sub_rule_counts": dict(sub_cnt.most_common(25)),
        "classes_qualifying_if_rule_off": loo,
        "field_kinds_qualifying": dict(fk),
        "member_access_total": len(rm),
        "member_access_dynamic": len(dyn_m),
        "member_access_class_receiver": len(cls_m),
        "member_access_qualifying_receiver": len(q_m),
        "member_access_qualifying_pct_of_total": pct(len(q_m), len(rm)),
        "member_access_qualifying_pct_of_dynamic": pct(len(q_m), len(dyn_m)),
        "member_access_qualifying_inside_functions": len(q_m_fn),
        "modules_considered": len(mods),
        "modules_ge50pct_v0": len(mods_ge50),
        "modules_ge50pct_v3": len(mods_ge50_v3),
        "largest_modules_ge50pct_v3": [{"file": m[0], "functions": m[1], "body_bytes": m[2], "v0_pct": pct(m[3], m[2]), "v3_pct": pct(m[4], m[2])} for m in top_mods],
        "largest_modules_ge50pct_v0": [{"file": m[0], "functions": m[1], "body_bytes": m[2], "v0_pct": pct(m[3], m[2])} for m in sorted(mods_ge50, key=lambda m: -m[2])[:3]],
        "function_body_bytes": body_all,
        "function_body_bytes_qualifying_pct": pct(body_q, body_all),
        "function_body_bytes_qualifying_v3_pct": pct(body_qg, body_all),
        "member_access_sound_in_region": sound_m,
        "member_access_sound_in_region_pct_of_dynamic": pct(sound_m, len(dyn_m)),
        "member_access_sound_in_region_v3": sound_m_g,
        "functions_total": len(rf),
        "functions_qualifying_t2": len(rq),
        "functions_qualifying_t2_pct": pct(len(rq), len(rf)),
        "functions_qualifying_t1": sum(1 for f in rf if f in q_t1),
        "functions_qualifying_guarded_casts": sum(1 for f in rf if f in qual_g),
        "functions_qualifying_guarded_casts_r4_lenient": sum(1 for f in rf if f in qual_g4),
        "functions_qualifying_v3_plus_generic_T_ok": sum(1 for f in rf if f in qual_g4t),
        "member_access_qualifying_v3_plus_generic_T_ok": sum(1 for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q_g4t),
        "member_access_qualifying_guarded_casts": sum(1 for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q_g),
        "member_access_qualifying_guarded_casts_r4_lenient": sum(1 for m in dyn_m if len(m["d"]) == 1 and m["d"][0] in Q_g4),
        "functions_qualifying_t0_strict": sum(1 for f in rf if f in q_t0),
        "functions_qualifying_nontrivial": len(nontrivial_q),
        "functions_by_kind": dict(kind_cnt),
        "functions_qualifying_by_kind": dict(kind_q),
        "function_fail_reasons": dict(freason.most_common()),
        "function_sole_fail_reasons": dict(fsole.most_common()),
        "boundary_call_sites": b_calls,
        "boundary_value_refs": b_vref,
        "boundary_escaping_closures": b_esc,
        "internal_call_sites": internal,
        "qualifying_functions_without_boundary": closed_regions,
        "qualifying_functions_exported": exported_q,
        "examples_qualifying_classes": q_examples,
        "examples_blocked_classes": b_examples,
        "examples_qualifying_functions": fq_examples,
        "r7_files": [file_of[fi] for fi in sorted(r7_files)],
        "iface_targets": len(iface_src),
        "iface_closed": sum(1 for i in iface_src if closed.get(i, True)),
    }
    regions = []
    rq3 = [f for f in rf if f in qual_g4t]
    for f in rq3:
        ff = funcs[f]
        p = file_of[ff["fi"]]
        v0 = f in qual
        regions.append({
            "v0": v0,
            "file": p, "name": ff["n"], "kind": ff["k"],
            "class": classes[ff["cls"]]["name"] if ff["cls"] in classes else None,
            "start": ff["s"], "end": ff["e"], "body_start": ff["bs"], "body_end": ff["be"],
            "line": LN.line(p, ff["s"]), "end_line": LN.line(p, max(ff["e"] - 1, ff["s"])),
            "tier": (fstat[f][1] if v0 else fstat_g4t[f][1]), "params": len(ff["params"]), "locals": len(ff["locals"]),
            "exported": ff["exp"], **(fboundary[f] if v0 else fboundary_v3[f]),
        })
    regions.sort(key=lambda r: (r["file"], r["start"]))

    # ---- hot agent: dump a verdict for every function (root and non-root files)
    def tdesc(t):
        if not isinstance(t, dict):
            return str(t)
        c = t.get("c")
        if c == "inst":
            return "class " + (classes[t["id"]]["name"] if t.get("id") in classes else t.get("fn", "?"))
        if c in ("iface", "tlit", "objlit"):
            return c + (" " + t["fn"] if "fn" in t else "") + (" #" + t["id"] if "id" in t else "")
        if c == "nullable":
            return tdesc(t.get("i", {})) + "|null"
        if c == "array":
            return tdesc(t.get("el", {})) + "[]"
        if c in ("union", "inter"):
            return c + "(" + ",".join(tdesc(m) for m in (t.get("m") or [])[:3]) + ")"
        return str(c)
    def first_bad(Qv, f):
        def ok(t):
            if t is None: return True
            c = t.get("c")
            if c in PRIM: return True
            if c == "inst" and t.get("id") in Qv: return True
            if c == "nullable":
                i = t.get("i", {}); return i.get("c") in PRIM or (i.get("c") == "inst" and i.get("id") in Qv)
            if c == "array" and "el" in t: return ok(t["el"])
            return False
        for p_ in f["params"]:
            if not ok(p_): return "param " + tdesc(p_)
        for l_ in f["locals"]:
            if not ok(l_): return "local " + tdesc(l_)
        return None
    allf = []
    for fid_, ff in funcs.items():
        p_ = file_of[ff["fi"]]
        rs0, t0 = fstat[fid_]
        rs3, t3 = fstat_g4t[fid_]
        d = {"id": fid_, "file": p_, "root": ff["fi"] in root_fi, "name": ff["n"], "kind": ff["k"],
             "s": ff["s"], "e": ff["e"], "bs": ff["bs"], "be": ff["be"], "line": LN.line(p_, ff["s"]),
             "nested": ff.get("nested"), "parent": ff.get("pid"), "ctx": ff.get("ctx"),
             "v0": not rs0, "v3": not rs3, "v0_reasons": rs0, "v3_reasons": rs3, "tier0": t0,
             "w_closed": not fstat_w1[fid_][0], "w_closed_tp": not fstat_w2[fid_][0], "w_v3_closed_tp": not fstat_w3[fid_][0],
             "bad_v0": first_bad(Q, ff), "bad_v3": first_bad(Q_g4t, ff)}
        cid = ff["cls"]
        if cid in classes:
            d["class"] = classes[cid]["name"]
            d["class_q_v0"] = cid in Q
            d["class_q_v3"] = cid in Q_g4t
            if cid not in Q:
                try:
                    d["class_rules"] = sorted(all_rules(cid))
                    d["class_ev"] = cls_ex(cid).get("evidence", [])[:5]
                except Exception as ex:
                    d["class_rules"] = ["?" + str(ex)]
        allf.append(d)
    os.makedirs(HOTOUT, exist_ok=True)
    json.dump(allf, open(os.path.join(HOTOUT, repo + ".allfuncs.json"), "w"))
    json.dump({"closed": {i: bool(closed.get(i, True)) for i in iface_src}, "src": {i: sorted(v) for i, v in iface_src.items()},
               "files": {str(k): v for k, v in file_of.items()}},
              open(os.path.join(HOTOUT, repo + ".ifaces.json"), "w"))
    qcls = [{"name": classes[c]["name"], "file": file_of[classes[c]["fi"]], "start": classes[c]["s"], "end": classes[c]["e"],
             "line": LN.line(file_of[classes[c]["fi"]], classes[c]["s"]), "fields": [f["n"] for f in classes[c]["fields"]]}
            for c in q_root]
    return S, regions, qcls


def pct(a, b):
    return round(100.0 * a / b, 1) if b else None


def main():
    repos = sys.argv[1:] or ["awfy", "typescript", "hono", "vscode", "zod", "datefns", "pixijs", "preact"]
    allreg = {}
    for repo in repos:
        S, regions, qcls = analyze(repo)
        json.dump(S, open(os.path.join(HOTOUT, repo + ".summary.json"), "w"), indent=1)
        allreg[repo] = {"qualifying_classes": qcls, "qualifying_functions": regions}
        print(json.dumps({k: v for k, v in S.items() if not k.startswith("examples")}, indent=None)[:3000])
        print()
    if len(repos) > 1:
        meta = {
            "about": "Sound-region finder v0 (static, rules r1-r7). Spans are UTF-8 byte offsets into the file text after "
                     "BOM stripping; start = first token of the function node incl. modifiers/decorators (tsrs skip_trivia(pos)); "
                     "body_start/body_end = the body block (or the expression body of an arrow).",
            "v0": "true = qualifies under the strict v0 rules (r1-r7 as written); false = qualifies only under v3 (`!` and class-to-class casts become guards, r4 ignores this.method() leaks and optional/undefined-typed fields, unconstrained generic T is not an escape). Boundary counts are relative to the same variant.",
            "tiers": {"0": "params/locals are number|string|boolean|bigint or a qualifying class",
                      "1": "tier 0 plus nullable (T|null|undefined) of those", "2": "tier 1 plus arrays of those"},
            "boundary": "boundary_calls = resolved call sites from non-qualifying code; value_refs = function used as a value; "
                        "escaping_closure = arrow/function expression that is passed or stored (unknown callers); internal = call sites in qualifying functions",
        }
        json.dump({"meta": meta, "repos": allreg}, open(os.path.join(HOTOUT, "regions.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
