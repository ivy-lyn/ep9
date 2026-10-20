#!/usr/bin/env python3
"""AOT-usefulness metrics per file from an ep9_typecheck JSONL.

usage: precision.py <tsrs.jsonl> [<tsrs.jsonl> ...]
metrics over value expressions (not declaration names):
  any%      : nodes typed any/error/unknown
  mono%     : nodes with a 'precise' category (number/string/boolean/instance/array/function/class_ctor/void/null/undefined/bigint)
  num%      : nodes typed number (incl. literal/enum)
  member    : property accesses; slot% = receiver is a class instance and the field slot was found;
              res% = property symbol resolved; poly% = union receiver (synthetic prop); dts% = prop declared in .d.ts
  calls     : call/new; tgt% = resolved signature has a declaration; dts% = target in .d.ts/lib; virt% = method on class
"""
import json
import sys
from collections import Counter

PRECISE = {'number', 'string', 'boolean', 'bigint', 'instance', 'array', 'function', 'class_ctor', 'void', 'null',
           'undefined', 'tuple', 'symbol'}
VALUE_KINDS = {'Identifier', 'PropertyAccessExpression', 'ElementAccessExpression', 'CallExpression', 'NewExpression',
               'BinaryExpression', 'PrefixUnaryExpression', 'PostfixUnaryExpression', 'ThisKeyword',
               'ConditionalExpression', 'ParenthesizedExpression', 'NumericLiteral', 'AwaitExpression'}


def is_decl_name(n):
    s = n.get('sym')
    return False if s is None else False


def main():
    rows = []
    for path in sys.argv[1:]:
        tot = Counter()
        for line in open(path):
            d = json.loads(line)
            if 'file' not in d:
                continue
            c = Counter()
            for n in d['nodes']:
                if n['k'] not in VALUE_KINDS or 'h' not in n:
                    continue
                cat = n['h']['c']
                c['n'] += 1
                if cat in ('any', 'error', 'unknown'):
                    c['any'] += 1
                if cat in PRECISE or (cat == 'nullable' and n['h'].get('inner', {}).get('c') in PRECISE):
                    c['mono'] += 1
                if cat == 'number':
                    c['num'] += 1
                m = n.get('member')
                if m:
                    c['mem'] += 1
                    if 'slot' in m:
                        c['slot'] += 1
                    if 'sym' in m:
                        c['res'] += 1
                        if m['sym'].get('decl', {}).get('dts') or m['sym'].get('decl', {}).get('lib'):
                            c['mdts'] += 1
                        if 'method' in m['sym'].get('f', []):
                            c['mmeth'] += 1
                    if m.get('poly'):
                        c['poly'] += 1
                    if m['recv'].get('c') in ('any', 'error', 'unknown'):
                        c['recv_any'] += 1
                cl = n.get('call')
                if cl:
                    c['calls'] += 1
                    t = cl.get('target')
                    if t:
                        c['tgt'] += 1
                        if t.get('dts') or t.get('lib'):
                            c['cdts'] += 1
                    if cl.get('method_on'):
                        c['virt'] += 1
                    if cl.get('ncand', 1) > 1:
                        c['overload'] += 1
            tot.update(c)
            rows.append((path.split('/')[-1].replace('.jsonl', '') + ':' + d['file'].split('/')[-1], c))
        rows.append((path.split('/')[-1].replace('.jsonl', '') + ':TOTAL', tot))

    def p(a, b):
        return f"{100.0 * a / b:5.1f}" if b else "  -  "
    print(f"{'file':38} {'nodes':>7} {'any%':>5} {'mono%':>5} {'num%':>5} | {'member':>6} {'res%':>5} {'slot%':>5} {'poly%':>5} {'rAny%':>5} {'dts%':>5} | {'calls':>6} {'tgt%':>5} {'dts%':>5} {'virt%':>5} {'ovl%':>5}")
    for name, c in rows:
        print(f"{name[:38]:38} {c['n']:7} {p(c['any'], c['n'])} {p(c['mono'], c['n'])} {p(c['num'], c['n'])} | "
              f"{c['mem']:6} {p(c['res'], c['mem'])} {p(c['slot'], c['mem'])} {p(c['poly'], c['mem'])} {p(c['recv_any'], c['mem'])} {p(c['mdts'], c['mem'])} | "
              f"{c['calls']:6} {p(c['tgt'], c['calls'])} {p(c['cdts'], c['calls'])} {p(c['virt'], c['calls'])} {p(c['overload'], c['calls'])}")


if __name__ == '__main__':
    main()
