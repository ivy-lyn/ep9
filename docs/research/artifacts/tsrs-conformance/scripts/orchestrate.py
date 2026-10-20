"""Run t262h workers over the manifest in parallel, with crash attribution.

usage: orchestrate.py <manifest.jsonl> <worker-binary> <out-results.jsonl> [jobs]
"""
import json, subprocess, sys, concurrent.futures as cf

manifest, binary, out = sys.argv[1], sys.argv[2], sys.argv[3]
jobs = int(sys.argv[4]) if len(sys.argv) > 4 else 16
rows = [json.loads(l) for l in open(manifest)]
tasks = [(r["id"], r["path"], r["runs"]) for r in rows]
CH = 250
chunks = [tasks[i:i + CH] for i in range(0, len(tasks), CH)]


def run(chunk, parsers="tsrs,oxc"):
    inp = "".join(f"{i}\t{p}\t{m}\n" for i, p, m in chunk)
    pr = subprocess.run([binary, "worker", parsers], input=inp.encode(), capture_output=True)
    results, begun = {}, []
    for line in pr.stdout.decode("utf-8", "replace").split("\n"):
        if line.startswith("R\t"):
            o = json.loads(line[2:])
            results[o["id"]] = o
        elif line.startswith("B\t"):
            _, i, parser, mode = line.split("\t")
            begun.append((int(i), parser, mode))
    return pr.returncode, results, begun


def process(chunk):
    out, events = {}, []
    todo = list(chunk)
    while todo:
        rc, results, begun = run(todo)
        out.update(results)
        if rc == 0:
            break
        done_ids = set(results)
        if rc == 3:  # tsrs panic: restart after the last finished test
            last = max(i for i in done_ids)
            idx = next(k for k, t in enumerate(todo) if t[0] == last)
            events.append({"id": last, "event": "tsrs_panic_restart"})
            todo = todo[idx + 1:]
            continue
        # crash: find the test that began but did not finish
        crashed = [b for b in begun if b[0] not in done_ids]
        cid, cparser, cmode = crashed[-1] if crashed else (todo[0][0], "unknown", "?")
        events.append({"id": cid, "event": "crash", "parser": cparser, "mode": cmode, "rc": rc})
        idx = next(k for k, t in enumerate(todo) if t[0] == cid)
        other = "oxc" if cparser == "tsrs" else "tsrs"
        rc2, r2, _ = run([todo[idx]], other)
        rec = r2.get(cid, {"id": cid, "runs": []})
        rec["crash"] = {"parser": cparser, "mode": cmode, "rc": rc, "other_rc": rc2}
        out[cid] = rec
        todo = todo[idx + 1:]
    return out, events


all_res, all_events = {}, []
with cf.ThreadPoolExecutor(jobs) as ex:
    for res, ev in ex.map(process, chunks):
        all_res.update(res)
        all_events.extend(ev)
with open(out, "w") as f:
    for r in rows:
        rec = all_res.get(r["id"], {"id": r["id"], "missing": True})
        f.write(json.dumps(rec) + "\n")
with open(out + ".events.json", "w") as f:
    json.dump(all_events, f, indent=1)
print(f"results: {len(all_res)}/{len(rows)}; events: {len(all_events)}", file=sys.stderr)
