import json, sys, os
from collections import Counter
HERE = os.path.dirname(os.path.abspath(__file__))
src = open(os.path.join(HERE, 'analyze.py')).read()
src = src.replace("    return S, regions, qcls", "    global DBG; DBG = dict(locals())\n    return S, regions, qcls")
ns = {"__name__": "x", "__file__": os.path.join(HERE, "analyze.py")}
exec(compile(src, "analyze.py", "exec"), ns)
def run(repo):
    ns["analyze"](repo)
    return ns["DBG"]
