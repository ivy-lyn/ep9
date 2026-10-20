#!/usr/bin/env python3
"""Join the tsrs type side table (ep9_typecheck JSONL) with oxc_parser node spans (oxc_span_dump JSONL).

usage: join.py <tsrs.jsonl> <oxc.jsonl> <report.json> [--examples N]
Key: (file, start, end) in UTF-8 bytes. tsrs start = skip_trivia(pos); oxc span has no trivia.
"""
import json
import sys
from collections import Counter, defaultdict

# oxc kind -> compatible tsrs kinds
COMPAT = {
    'IdentifierReference': {'Identifier'},
    'BindingIdentifier': {'Identifier'},
    'IdentifierName': {'Identifier'},
    'LabelIdentifier': {'Identifier'},
    'PrivateIdentifier': {'PrivateIdentifier'},
    'ThisExpression': {'ThisKeyword'},
    'Super': {'SuperKeyword'},
    'ArrayExpression': {'ArrayLiteralExpression'},
    'ObjectExpression': {'ObjectLiteralExpression'},
    'TemplateLiteral': {'TemplateExpression', 'NoSubstitutionTemplateLiteral'},
    'TaggedTemplateExpression': {'TaggedTemplateExpression'},
    'ComputedMemberExpression': {'ElementAccessExpression'},
    'StaticMemberExpression': {'PropertyAccessExpression'},
    'PrivateFieldExpression': {'PropertyAccessExpression'},
    'CallExpression': {'CallExpression'},
    'NewExpression': {'NewExpression'},
    'ImportExpression': {'CallExpression'},
    'MetaProperty': {'MetaProperty'},
    'SpreadElement': {'SpreadElement', 'SpreadAssignment'},
    'UpdateExpression': {'PrefixUnaryExpression', 'PostfixUnaryExpression'},
    'UnaryExpression': {'PrefixUnaryExpression', 'TypeOfExpression', 'VoidExpression', 'DeleteExpression'},
    'BinaryExpression': {'BinaryExpression'},
    'PrivateInExpression': {'BinaryExpression'},
    'LogicalExpression': {'BinaryExpression'},
    'AssignmentExpression': {'BinaryExpression'},
    'SequenceExpression': {'BinaryExpression'},
    'ConditionalExpression': {'ConditionalExpression'},
    'ArrowFunctionExpression': {'ArrowFunction'},
    'FunctionExpression': {'FunctionExpression'},
    'ClassExpression': {'ClassExpression'},
    'ParenthesizedExpression': {'ParenthesizedExpression'},
    'AwaitExpression': {'AwaitExpression'},
    'YieldExpression': {'YieldExpression'},
    'StringLiteral': {'StringLiteral'},
    'NumericLiteral': {'NumericLiteral'},
    'BigIntLiteral': {'BigIntLiteral'},
    'BooleanLiteral': {'TrueKeyword', 'FalseKeyword'},
    'NullLiteral': {'NullKeyword'},
    'RegExpLiteral': {'RegularExpressionLiteral'},
    'TSAsExpression': {'AsExpression'},
    'TSSatisfiesExpression': {'SatisfiesExpression'},
    'TSNonNullExpression': {'NonNullExpression'},
    'TSTypeAssertion': {'TypeAssertionExpression'},
    'TSInstantiationExpression': {'ExpressionWithTypeArguments'},
    'ChainExpression': None,  # no tsrs node; same span as its inner expression
    'JSXElement': {'JsxElement', 'JsxSelfClosingElement'},
}
# The "AOT-relevant" core: what a compiler needs a type for.
CORE = {'IdentifierReference', 'StaticMemberExpression', 'ComputedMemberExpression', 'PrivateFieldExpression',
        'CallExpression', 'NewExpression', 'BinaryExpression', 'LogicalExpression', 'AssignmentExpression',
        'UnaryExpression', 'UpdateExpression', 'ThisExpression', 'ConditionalExpression', 'ArrayExpression',
        'ObjectExpression', 'NumericLiteral', 'StringLiteral', 'TemplateLiteral', 'ArrowFunctionExpression',
        'FunctionExpression', 'AwaitExpression', 'TSAsExpression', 'TSNonNullExpression', 'ParenthesizedExpression'}


def load(path):
    out = {}
    stats = None
    for line in open(path):
        d = json.loads(line)
        if 'stats' in d:
            stats = d['stats']
            continue
        out[d['file']] = d
    return out, stats


def main():
    tsrs_path, oxc_path, report_path = sys.argv[1:4]
    nex = int(sys.argv[sys.argv.index('--examples') + 1]) if '--examples' in sys.argv else 8
    tsrs, tstats = load(tsrs_path)
    oxc, _ = load(oxc_path)
    per_kind = defaultdict(Counter)  # oxc kind -> outcome counts
    mism_pairs = Counter()
    nospan_reason = Counter()
    examples = defaultdict(list)
    rev_unmatched = Counter()
    typed_core = Counter()
    hint_cats = Counter()
    any_by_kind = Counter()
    texts = {}
    files_missing = []
    for f, od in oxc.items():
        td = tsrs.get(f)
        if td is None:
            files_missing.append(f)
            continue
        src = open(f, 'rb').read()
        if src.startswith(b'\xef\xbb\xbf'):
            src = src[3:]
        texts[f] = src
        idx = defaultdict(list)
        by_start = defaultdict(list)
        by_end = defaultdict(list)
        for r in td['nodes']:
            idx[(r['s'], r['e'])].append(r)
            by_start[r['s']].append(r)
            by_end[r['e']].append(r)
        matched_tsrs = set()
        for kind, s, e, in_type, parent in od['nodes']:
            if kind not in COMPAT or in_type:
                continue
            if kind == 'FunctionExpression' and parent in ('MethodDefinition', 'ObjectPropertyMethod', 'TSAbstractMethodDefinition'):
                continue  # method body holder, not a JS expression on the tsrs side
            if kind in ('IdentifierName',) and parent in ('ImportSpecifier', 'ExportSpecifier'):
                pass
            cands = idx.get((s, e), [])
            want = COMPAT[kind]
            if want is None:
                hit = cands[:1]
            else:
                hit = [r for r in cands if r['k'] in want]
            if hit:
                r = hit[0]
                outcome = 'match'
                matched_tsrs.add(id(r))
                if 'panic' in r:
                    outcome = 'match_panic'
                elif 'h' in r:
                    c = r['h'].get('c')
                    if kind in CORE:
                        hint_cats[c] += 1
                        if c in ('any', 'error'):
                            any_by_kind[kind] += 1
                per_kind[kind][outcome] += 1
            elif cands:
                per_kind[kind]['kind_mismatch'] += 1
                key = (kind, ','.join(sorted({r['k'] for r in cands})))
                mism_pairs[key] += 1
                if len(examples['km:' + kind]) < nex:
                    examples['km:' + kind].append((f.split('/')[-1], s, e, src[s:e][:60].decode('utf-8', 'replace'), key[1]))
            else:
                per_kind[kind]['no_span'] += 1
                ss = [r['k'] for r in by_start.get(s, [])]
                ee = [r['k'] for r in by_end.get(e, [])]
                if ss and ee:
                    why = 'start&end exist separately'
                elif ss:
                    why = 'same start, different end: ' + ','.join(sorted(set(ss)))[:60]
                elif ee:
                    why = 'same end, different start: ' + ','.join(sorted(set(ee)))[:60]
                else:
                    why = 'nothing at start or end'
                nospan_reason[(kind, why)] += 1
                if len(examples['ns:' + kind]) < nex:
                    examples['ns:' + kind].append((f.split('/')[-1], s, e, src[s:e][:60].decode('utf-8', 'replace'), why))
        for r in td['nodes']:
            if id(r) not in matched_tsrs:
                rev_unmatched[r['k']] += 1
    # summarise
    rows = []
    tot = Counter()
    core_tot = Counter()
    for k, c in sorted(per_kind.items(), key=lambda kv: -sum(kv[1].values())):
        n = sum(c.values())
        m = c['match'] + c['match_panic']
        rows.append({'oxc_kind': k, 'n': n, 'match': m, 'pct': round(100.0 * m / n, 2),
                     'kind_mismatch': c['kind_mismatch'], 'no_span': c['no_span'], 'panic': c['match_panic']})
        tot.update(c)
        if k in CORE:
            core_tot.update(c)
    def pct(c):
        n = sum(c.values())
        return round(100.0 * (c['match'] + c['match_panic']) / n, 3) if n else None
    report = {
        'files': len(oxc), 'files_missing_in_tsrs': files_missing,
        'overall': {'n': sum(tot.values()), 'match_pct': pct(tot), **tot},
        'core': {'n': sum(core_tot.values()), 'match_pct': pct(core_tot), **core_tot},
        'by_kind': rows,
        'kind_mismatch_pairs': [[a, b, n] for (a, b), n in mism_pairs.most_common(30)],
        'no_span_reasons': [[a, b, n] for (a, b), n in nospan_reason.most_common(30)],
        'tsrs_unmatched_by_kind': rev_unmatched.most_common(30),
        'core_hint_categories': hint_cats.most_common(),
        'core_any_by_oxc_kind': any_by_kind.most_common(),
        'examples': {k: v for k, v in examples.items()},
        'tsrs_stats': tstats,
    }
    json.dump(report, open(report_path, 'w'), indent=1)
    print(f"files={report['files']} overall n={report['overall']['n']} match={report['overall']['match_pct']}% "
          f"core n={report['core']['n']} match={report['core']['match_pct']}%")
    for r in rows[:40]:
        print(f"  {r['oxc_kind']:28} n={r['n']:7} match={r['pct']:6}% km={r['kind_mismatch']:5} nospan={r['no_span']:5} panic={r['panic']}")
    print('kind mismatches:', report['kind_mismatch_pairs'][:12])
    print('no-span reasons:', report['no_span_reasons'][:12])
    print('tsrs unmatched:', report['tsrs_unmatched_by_kind'][:15])
    tot_h = sum(hint_cats.values())
    print('core hint cats:', [(c, n, round(100 * n / tot_h, 1)) for c, n in hint_cats.most_common()])


if __name__ == '__main__':
    main()
