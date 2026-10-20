"""Assign each tsrs false negative a fix-location category. usage: categorize.py <outdir>"""
import csv, json, os, re, sys, collections
od = sys.argv[1]
rows = list(csv.DictReader(open(os.path.join(od, "tsrs_false_negatives.csv"))))

RULES = [  # (regex on normalized oxc message, category, short label)
    (r"^Unterminated regular expression$", "SCANNER", "LS/PS inside regex literal accepted by tsrs scanner"),
    (r"^Invalid regular expression|regular expression literal$", "REGEX", "regex pattern/flags"),
    (r"has already been declared|Label `X` has already|Multiple constructor", "SCOPE", "redeclaration / duplicate binding"),
    (r"undefined label|Jump target|Illegal break|Illegal continue|can only jump to a label", "SCOPE", "label / break / continue target"),
    (r"must be declared in an enclosing class|not allowed outside class bodies", "SCOPE", "private name resolution"),
    (r"Super calls|can only be referenced in members|can only be referenced in a derived", "SCOPE", "super() / super.x context"),
    (r"in class field initializer|in static initialization block|await in class static", "SCOPE", "arguments/await in class field or static block"),
    (r"Export 'X' is not defined|Duplicated export|multiple default exports", "SCOPE", "module export table"),
    (r"The keyword 'X' is reserved|Cannot assign to 'X' in strict mode|Delete of an unqualified|'X' statements are not allowed|Invalid function declaration", "SCOPE", "strict-mode-dependent rule"),
    (r"as an identifier in an async context|as an identifier in a generator context|only allowed within async functions|not allowed in formal parameter|only allowed in a generator body", "SCOPE", "yield/await context"),
    (r"Unexpected new.target|Unexpected import.meta|statement can only be used within a function body", "SCOPE", "function/goal context (new.target, return, import.meta)"),
    (r"Cannot use (import|export) statement outside a module|can only be used at the top level of a module|declarations are not allowed at the top level of a script", "GOAL", "Script vs Module goal / top-level position"),
    (r"Cannot assign to this expression|Invalid rest element target|Invalid assignment in object literal|Private identifier 'X' is not allowed in property names|Unexpected private identifier|^Unexpected token$", "AST", "invalid assignment / destructuring target"),
    (r"rest element|rest parameter|Spread must be last", "AST", "rest element/param rules"),
    (r"operand of a 'X' operator cannot be a private identifier|Private fields cannot be accessed on super|Classes can|Classes may not|Constructor can't", "AST", "class element rules"),
    (r"Illegal 'X' directive in function with non-simple", "AST", "'use strict' + non-simple params"),
    (r"import\(\)|dynamic import|Dynamic imports|valid property accesses on import|Expected `X` but found `X`", "AST", "import()/import.<name> forms (tsrs builds MetaProperty for any name)"),
    (r"Lexical declaration cannot appear|can only be declared at the top level or inside a block|Invalid class declaration|bare case statement", "AST", "declaration in single-statement position"),
    (r"Using declarations|using declaration|Missing initializer in const|loop variable declaration may not have an initializer|Only a single declaration|cannot be declared as a variable name inside", "AST", "declaration-list rules (const/using/for-in-of)"),
    (r"Tagged template expressions are not permitted|Logical expressions and coalesce|accessor must not have|clause cannot appear more than once|modifier cannot be used here|left-hand side of a `X` statement may not be|statement cannot be used inside a class static block|export name cannot include|string literal cannot be used as an exported", "AST", "misc static-semantics rule"),
    (r"Expected a semicolon", "AST", "let-declaration in single-statement position / rest-not-final (ASI-shaped)"),
    (r"Line terminator not permitted|Illegal newline after throw|Keywords cannot contain escape", "AST", "needs token/trivia check (line break before =>, throw\\n, escaped keyword)"),
]

def classify(msg):
    for rx, cat, lab in RULES:
        if re.search(rx, msg or ""):
            return cat, lab
    return "UNCLASSIFIED", msg

for r in rows:
    r["cat"], r["label"] = classify(r["oxc_cluster"])
with open(os.path.join(od, "tsrs_false_negatives_categorized.csv"), "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys())); w.writeheader(); w.writerows(rows)

tot = len(rows)
print("category totals (tsrs parser-only false negatives, main scope):")
cc = collections.Counter(r["cat"] for r in rows)
for k, v in cc.most_common():
    bf = sum(r["tsrs_binder_f"] == "True" for r in rows if r["cat"] == k)
    ba = sum(r["tsrs_binder_any"] == "True" for r in rows if r["cat"] == k)
    rx = sum(r["tsrs_regex"] == "True" for r in rows if r["cat"] == k)
    ow = collections.Counter(r["oxc_where"] for r in rows if r["cat"] == k)
    print(f"  {k:13} {v:5} ({100*v/tot:4.1f}%)  binderF={bf} binderAny={ba} regex={rx} oxc_where={dict(ow)}")
print("\nby label:")
lc = collections.Counter((r["cat"], r["label"]) for r in rows)
summary = []
for (c, l), v in lc.most_common():
    ex = [r["rel"] for r in rows if r["label"] == l][:2]
    print(f"  {v:5} {c:12} {l} | {ex[0]}")
    summary.append({"cat": c, "label": l, "count": v, "examples": ex})
json.dump({"category_totals": dict(cc), "labels": summary}, open(os.path.join(od, "tsrs_fn_categories.json"), "w"), indent=1)
