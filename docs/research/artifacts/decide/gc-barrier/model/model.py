"""Exhaustive interleaving model of one mutator and one concurrent marker (H11 decision).

Sequential consistency between sub-steps. This is what the SeqCst fences buy: the mutator's
`store; fence; load state` and the marker's `set BLACK; fence; load field` behave as SC sub-steps.

Barrier forms (B = barrier, M = marking):
  rw_slowstore : decided form. pre-check is_cell(new) && state <= MARKING (always true while
                 marking), slow path: store; then re-check state == BLACK -> GREY + push.
                 Allocation WHITE. Termination: STW root rescan + drain to fixpoint.
  rw_precheck  : wrong variant: check state BEFORE the store, with no re-check after it.
  rw_norescan  : decided barrier, but no root rescan at termination.
  rw_oldfilter : retreating wavefront that filters on the OLD value (old-only form).
  satb_new     : Yuasa SATB (shade old value), filter on NEW value only (03-gc text).
  satb_oldnew  : Yuasa SATB, filter old OR new (prototype). Allocation BLACK. No rescan.
  satb_noweakrb: satb_oldnew, but a weak read (WeakRef.deref) has no read barrier.
  rw_weak      : decided form with the same weak read and no read barrier.
"""
import sys
from functools import lru_cache

INT = ('int', 5)

def is_cell(v):
    return isinstance(v, str)

SCENARIOS = {
    # Move the only reference to C from A.f to B.g through a register.
    'move': dict(
        heap={'R': {'a': 'A', 'b': 'B'}, 'A': {'f': 'C'}, 'B': {'g': INT}, 'C': {}},
        roots=('R',),
        prog=[('load', 'x', 'A', 'f'), ('store', 'A', 'f', INT), ('store', 'B', 'g', 'x'), ('drop', 'x')],
    ),
    # Allocate N during marking, put C into N (fresh store), publish N into B.
    'alloc': dict(
        heap={'R': {'a': 'A', 'b': 'B'}, 'A': {'f': 'C'}, 'B': {'g': INT}, 'C': {}},
        roots=('R',),
        prog=[('load', 'x', 'A', 'f'), ('new', 'y', 'N'), ('store', 'N', 'h', 'x'),
              ('store', 'A', 'f', INT), ('store', 'B', 'g', 'y'), ('drop', 'x'), ('drop', 'y')],
    ),
    # Keep C only in a register (root) after deleting the heap edge.
    'reg': dict(
        heap={'R': {'a': 'A'}, 'A': {'f': 'C'}, 'C': {}},
        roots=('R',),
        prog=[('load', 'x', 'A', 'f'), ('store', 'A', 'f', INT)],
    ),
    # N escapes into B first; then the mutator stores C into N (a later, non-initializing store).
    'escape': dict(
        heap={'R': {'a': 'A', 'b': 'B'}, 'A': {'f': 'C'}, 'B': {'g': INT}, 'C': {}},
        roots=('R',),
        prog=[('new', 'y', 'N'), ('store', 'B', 'g', 'y'), ('drop', 'y'), ('load', 'x', 'A', 'f'),
              ('store', 'A', 'f', INT), ('load', 'y', 'B', 'g'), ('store', 'N', 'h', 'x'), ('drop', 'x'), ('drop', 'y')],
    ),
    # W is held only by a weak cell; the mutator derefs it and stores it into B.
    'weak': dict(
        heap={'R': {'b': 'B'}, 'B': {'g': INT}, 'W': {}},
        roots=('R',),
        weak={'wk': 'W'},
        prog=[('wload', 'x', 'wk'), ('store', 'B', 'g', 'x'), ('drop', 'x')],
    ),
}

FORMS = ['rw_slowstore', 'rw_precheck', 'rw_norescan', 'rw_oldfilter', 'rw_elide_escaped',
         'satb_new', 'satb_oldnew', 'satb_noweakrb', 'rw_weak']

def run(form, sc):
    heap0 = tuple(sorted((o, tuple(sorted(f.items()))) for o, f in sc['heap'].items()))
    prog = sc['prog']
    weak = sc.get('weak', {})
    satb = form.startswith('satb')
    rescan = form in ('rw_slowstore', 'rw_precheck', 'rw_oldfilter', 'rw_weak', 'rw_elide_escaped')
    weak_rb = form in ('satb_new', 'satb_oldnew')  # SATB forms with a weak read barrier

    # Initial STW pause: scan roots -> grey.
    init_grey = frozenset(sc['roots'])
    failures = []
    seen = set()

    def H(heap):
        return {o: dict(f) for o, f in heap}

    def freeze(h):
        return tuple(sorted((o, tuple(sorted(f.items()))) for o, f in h.items()))

    # state: heap, regs, pc, marked, black(set), grey(worklist set), visiting (obj, fields left), pending
    # pending: mutator sub-step for the decided form: ('recheck', host)
    def step_all(heap, regs, pc, marked, black, grey, visiting, pending, newobjs):
        key = (heap, regs, pc, marked, black, grey, visiting, pending, newobjs)
        if key in seen:
            return
        seen.add(key)
        h = H(heap)
        regs_d = dict(regs)
        mut_done = pc >= len(prog) and pending is None
        mark_done = not grey and visiting is None
        if mut_done and mark_done:
            finish(h, regs_d, set(marked), set(newobjs))
            return
        # ---- marker steps ----
        if visiting is not None:
            obj, left = visiting
            if not left:
                step_all(heap, regs, pc, marked, black, grey, None, pending, newobjs)
            else:
                f = left[0]
                v = h[obj].get(f)
                m2, g2 = set(marked), set(grey)
                if is_cell(v) and v not in m2:
                    m2.add(v); g2.add(v)
                step_all(heap, regs, pc, frozenset(m2), black, frozenset(g2), (obj, left[1:]), pending, newobjs)
        elif grey:
            for o in sorted(grey):
                g2 = set(grey); g2.discard(o)
                b2 = set(black); b2.add(o)  # set BLACK, then (fence) read fields one by one
                step_all(heap, regs, pc, marked, frozenset(b2), frozenset(g2),
                         (o, tuple(sorted(h[o].keys()))), pending, newobjs)
        # ---- mutator steps ----
        if pending is not None and pending[0] == 'store_after':
            o, f, v = pending[1]
            h[o][f] = v
            step_all(freeze(h), regs, pc + 1, marked, black, grey, visiting, None, newobjs)
            return
        if pending is not None:
            kind, host = pending
            b2, g2 = set(black), set(grey)
            if host in b2:          # re-check after store + fence: BLACK -> GREY, push
                b2.discard(host); g2.add(host)
            step_all(heap, regs, pc, marked, frozenset(b2), frozenset(g2), visiting, None, newobjs)
            return
        if pc >= len(prog):
            return
        op = prog[pc]
        m2, g2, b2 = set(marked), set(grey), set(black)
        if op[0] == 'load':
            _, r, o, f = op
            regs_d[r] = h[o][f]
            step_all(heap, tuple(sorted(regs_d.items())), pc + 1, marked, black, grey, visiting, None, newobjs)
        elif op[0] == 'wload':
            _, r, wk = op
            v = weak[wk]
            regs_d[r] = v
            if satb and weak_rb and v not in m2:
                m2.add(v); g2.add(v)
            step_all(heap, tuple(sorted(regs_d.items())), pc + 1, frozenset(m2), black, frozenset(g2), visiting, None, newobjs)
        elif op[0] == 'drop':
            regs_d.pop(op[1], None)
            step_all(heap, tuple(sorted(regs_d.items())), pc + 1, marked, black, grey, visiting, None, newobjs)
        elif op[0] == 'new':
            _, r, name = op
            h[name] = {}
            regs_d[r] = name
            n2 = set(newobjs); n2.add(name)
            if satb:          # allocate black
                m2.add(name); b2.add(name)
            step_all(freeze(h), tuple(sorted(regs_d.items())), pc + 1, frozenset(m2), frozenset(b2), grey, visiting, None, frozenset(n2))
        elif op[0] == 'store':
            _, o, f, src = op
            v = regs_d[src] if isinstance(src, str) else src
            old = h[o].get(f)
            if is_cell(v) and v in newobjs:
                newobjs = frozenset(set(newobjs) | {v + '!'})
            escaped = (o + '!') in newobjs
            fresh = o in newobjs and (form == 'rw_elide_escaped' or not escaped)  # elision E2
            if satb:
                fire = (is_cell(v) if form == 'satb_new' else (is_cell(old) or is_cell(v)))
                if fire and o not in newobjs and is_cell(old) and old not in m2:
                    m2.add(old); g2.add(old)   # Yuasa: shade the old value
                h[o][f] = v
                step_all(freeze(h), regs, pc + 1, frozenset(m2), black, frozenset(g2), visiting, None, newobjs)
            else:
                fire = is_cell(old) if form == 'rw_oldfilter' else is_cell(v)
                if fresh:
                    fire = False
                if form == 'rw_precheck' and fire:
                    # check BEFORE the store: if host is BLACK now, regrey; the store is a later step
                    if o in b2:
                        b2.discard(o); g2.add(o)
                    step_all(heap, regs, pc, marked, frozenset(b2), frozenset(g2), visiting, ('store_after', (o, f, v)), newobjs)
                elif fire:
                    h[o][f] = v      # slow path: store, fence, then re-check (separate SC step)
                    step_all(freeze(h), regs, pc + 1, marked, black, grey, visiting, ('recheck', o), newobjs)
                else:
                    h[o][f] = v
                    step_all(freeze(h), regs, pc + 1, marked, black, grey, visiting, None, newobjs)

    def finish(h, regs_d, marked, newobjs):
        # STW termination.
        if rescan:
            work = [r for r in list(sc['roots']) + [v for v in regs_d.values() if is_cell(v)] if r not in marked]
            for r in work:
                marked.add(r)
            while work:
                o = work.pop()
                for v in h[o].values():
                    if is_cell(v) and v not in marked:
                        marked.add(v); work.append(v)
        live = set()
        stack = list(sc['roots']) + [v for v in regs_d.values() if is_cell(v)]
        while stack:
            o = stack.pop()
            if o in live:
                continue
            live.add(o)
            stack.extend(v for v in h[o].values() if is_cell(v))
        kept = marked | ({n for n in newobjs if not n.endswith('!')} if satb else set())
        lost = live - kept
        if lost:
            failures.append(sorted(lost))

    regs0 = ()
    step_all(heap0, regs0, 0, init_grey, frozenset(), init_grey, None, None, frozenset())
    return len(seen), failures

if __name__ == '__main__':
    print(f"{'form':14} " + ' '.join(f'{s:>14}' for s in SCENARIOS))
    for form in FORMS:
        row = []
        for name, sc in SCENARIOS.items():
            n, fails = run(form, sc)
            row.append(('LOST ' + ','.join(fails[0]) if fails else 'ok') + f'/{n}')
        print(f'{form:14} ' + ' '.join(f'{c:>14}' for c in row))
