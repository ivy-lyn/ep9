import math
def sp(s,g): return 1/((1-s)+s/g)
def pct(x): return round((x-1)*100,2)
def gm(xs): return math.exp(sum(math.log(x) for x in xs)/len(xs))
# hot shares (self+bi, % of all samples) from SR/hot/res/table.md
tsc=(0.067,0.084,0.088); vsc=(0.239,0.391,0.434); hono=(0.002,0.062,0.062); dfn=(0.059,0.110,0.112); awfy=(0.644,0.993)
print("A) reviewer check, g=1.27 everywhere")
lo=[sp(tsc[0],1.27),sp(vsc[0],1.27),sp(hono[0],1.27)]; awlo=sp(awfy[0],1.27)
print(" low: tsc,vsc,hono,awfy", [pct(x) for x in lo+[awlo]], "gm4", pct(gm(lo+[awlo])), "gm3", pct(gm(lo)))
hi=[sp(tsc[1],1.27),sp(vsc[1],1.27),sp(hono[1],1.27)]; awhi=sp(awfy[1],1.27)
print(" high v0: ", [pct(x) for x in hi+[awhi]], "gm4", pct(gm(hi+[awhi])), "gm3", pct(gm(hi)))
hi3=[sp(tsc[2],1.27),sp(vsc[2],1.27),sp(hono[2],1.27)]
print(" high v3: gm3", pct(gm(hi3)))
print("B) revised: tsc/hono/date-fns g=1.02-1.08 (guard-only band); vscode-diff g=1.02-1.27; kernels g<=1.27")
for name,s in [("tsc",tsc),("hono",hono),("date-fns",dfn)]:
    print(" ",name, pct(sp(s[0],1.02)), "to", pct(sp(s[2],1.08)))
print("  vscode-diff", pct(sp(vsc[0],1.02)), "to", pct(sp(vsc[2],1.27)), " (at 1.08 high:", pct(sp(vsc[2],1.08)),")")
print("  kernels v3 upper", pct(sp(awfy[0],1.27)), "to", pct(sp(awfy[1],1.27)))
lo=[sp(tsc[0],1.02),sp(vsc[0],1.02),sp(hono[0],1.02)]
hi=[sp(tsc[2],1.08),sp(vsc[2],1.27),sp(hono[2],1.08)]
print("  gm3 real apps:", pct(gm(lo)), "to", pct(gm(hi)))
print("C) old §4 row values check, g=1.27: tsc", pct(sp(.067,1.27)), pct(sp(.088,1.27)), "date-fns", pct(sp(.059,1.27)), pct(sp(.112,1.27)), "vsc", pct(sp(.239,1.27)), pct(sp(.434,1.27)))
print("D) JIT 1.05: tsc", pct(sp(.067,1.05)), pct(sp(.088,1.05)), "vsc", pct(sp(.239,1.05)), pct(sp(.434,1.05)))
print("E) g=1.27 everywhere, high end with v3 shares (tsc .088, vsc .434, hono .062, awfy .993)")
hi3=[sp(.088,1.27),sp(.434,1.27),sp(.062,1.27)]
print("  gm3", pct(gm(hi3)), "gm4", pct(gm(hi3+[sp(.993,1.27)])))
