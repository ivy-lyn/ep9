#!/bin/zsh
H=$SR/hot
cd $H
python3 -I hotmap.py tsc typescript res/tsc.json prof/tsc/run*.cpuprofile > res/tsc.txt
python3 -I hotmap.py tsc-noinl typescript res/tsc-noinl.json prof/tsc-noinl/run*.cpuprofile > res/tsc-noinl.txt
for f in richards deltablue nbody; do
  python3 -I hotmap.py awfy-$f awfy res/awfy-$f.json prof/awfy/$f.cpuprofile > res/awfy-$f.txt
  python3 -I hotmap.py awfy-$f-noinl awfy res/awfy-$f-noinl.json prof/awfy-noinl/$f.cpuprofile > res/awfy-$f-noinl.txt
done
for w in hono-fetch:hono hono-routers:hono zod-parse:zod datefns-fmtparse:datefns; do
  n=${w%%:*}; repo=${w#*:}
  python3 -I hotmap.py $n $repo res/$n.json prof/lib/$n.cpuprofile > res/$n.txt
  python3 -I hotmap.py $n-noinl $repo res/$n-noinl.json prof/lib-noinl/$n.cpuprofile > res/$n-noinl.txt
done
for m in "" -noinl; do d=lib$m; python3 -I hotmap.py vscode-diff$m vscode res/vscode-diff$m.json prof/$d/vscode-diff.cpuprofile > res/vscode-diff$m.txt; done
