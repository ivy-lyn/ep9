"""Critic check for H11: large arrays (LOS) use cards in the S2 slow path.
Decided text (03 §3 / H11-slowpath-S2): slow path = store(Release); fence; if host is LOS:
dirty_card(slot); return.  Marker: clear card; fence; scan the card range.  Termination rescans:
CSS, VM stack, Global, ScratchRoots, unit RW link cells, embedder roots, mutator mark stack,
weak/ephemeron constraints.  Dirty cards are NOT in that list.
Exhaustive SC interleavings of one mutator and one marker (same abstraction as gc-barrier/model.py).
Forms:
  decided      : no card rescan at termination, LOS host never pushed
  fix_rescan   : termination also rescans dirty cards of marked LOS hosts
  fix_push     : slow path also greys+pushes the LOS host once (card revisit)
"""
INT = ('int', 0)
def is_cell(v): return isinstance(v, str)
HEAP = {'R': {'a': 'A', 'l': 'L'}, 'A': {'f': 'C'}, 'L': {'e0': INT, 'e1': INT}, 'C': {}}
LOS = {'L'}
PROG = [('load', 'x', 'A', 'f'), ('store', 'L', 'e1', 'x'), ('store', 'A', 'f', INT), ('drop', 'x')]
ROOTS = ('R',)

def run(form):
    seen, fails = set(), []
    def fz(h): return tuple(sorted((o, tuple(sorted(f.items()))) for o, f in h.items()))
    def H(h): return {o: dict(f) for o, f in h}
    def step(heap, regs, pc, marked, black, grey, visiting, pending, dirty):
        key = (heap, regs, pc, marked, black, grey, visiting, pending, dirty)
        if key in seen: return
        seen.add(key)
        h, rd = H(heap), dict(regs)
        if pc >= len(PROG) and pending is None and not grey and visiting is None:
            finish(h, rd, set(marked), set(dirty)); return
        # marker
        if visiting is not None:
            o, left = visiting
            if not left:
                step(heap, regs, pc, marked, black, grey, None, pending, dirty)
            else:
                v = h[o][left[0]]; m2, g2 = set(marked), set(grey)
                if is_cell(v) and v not in m2: m2.add(v); g2.add(v)
                step(heap, regs, pc, frozenset(m2), black, frozenset(g2), (o, left[1:]), pending, dirty)
        elif grey:
            for o in sorted(grey):
                g2 = set(grey); g2.discard(o); b2 = set(black); b2.add(o); d2 = set(dirty)
                d2.discard(o)  # LOS: clear cards, fence, then scan (whole object here)
                step(heap, regs, pc, marked, frozenset(b2), frozenset(g2), (o, tuple(sorted(h[o]))), pending, frozenset(d2))
        # mutator
        if pending is not None:
            kind, o = pending
            d2, g2, b2 = set(dirty), set(grey), set(black)
            if o in LOS:
                d2.add(o)
                if form == 'fix_push' and o in b2: b2.discard(o); g2.add(o)
            elif o in b2: b2.discard(o); g2.add(o)
            step(heap, regs, pc, marked, frozenset(b2), frozenset(g2), visiting, None, frozenset(d2)); return
        if pc >= len(PROG): return
        op = PROG[pc]
        if op[0] == 'load':
            rd[op[1]] = h[op[2]][op[3]]
            step(heap, tuple(sorted(rd.items())), pc + 1, marked, black, grey, visiting, None, dirty)
        elif op[0] == 'drop':
            rd.pop(op[1], None)
            step(heap, tuple(sorted(rd.items())), pc + 1, marked, black, grey, visiting, None, dirty)
        elif op[0] == 'store':
            _, o, f, src = op
            v = rd[src] if isinstance(src, str) else src
            h[o][f] = v
            nxt = ('recheck', o) if is_cell(v) else None   # MARKING: every cell store -> slow path
            step(fz(h), regs, pc + 1, marked, black, grey, visiting, nxt, dirty)
    def finish(h, rd, marked, dirty):
        work = [r for r in list(ROOTS) + [v for v in rd.values() if is_cell(v)] if r not in marked]
        if form == 'fix_rescan':
            work += [o for o in dirty if o in marked]
        for r in work: marked.add(r)
        while work:
            o = work.pop()
            for v in h[o].values():
                if is_cell(v) and v not in marked: marked.add(v); work.append(v)
        live, st = set(), list(ROOTS) + [v for v in rd.values() if is_cell(v)]
        while st:
            o = st.pop()
            if o in live: continue
            live.add(o); st.extend(v for v in h[o].values() if is_cell(v))
        lost = live - marked
        if lost: fails.append(sorted(lost))
    step(fz(HEAP), (), 0, frozenset(ROOTS), frozenset(), frozenset(ROOTS), None, None, frozenset())
    return len(seen), fails

for form in ('decided', 'fix_rescan', 'fix_push'):
    n, f = run(form)
    print(f"{form:11} states={n:4} {'LOST ' + ','.join(f[0]) + f' ({len(f)} bad end states)' if f else 'ok'}")
