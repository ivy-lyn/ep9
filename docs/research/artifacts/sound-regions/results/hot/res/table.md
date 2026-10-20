| workload | mode | samples | Q-v0 self+bi | Q-v3 self+bi | not-Q (analyzed) | other JS (node/eval/driver) | VM gc+program | Q-v0 incl (any frame) | v0 % of analyzed | static fn share v0/v3 |
|---|---|---|---|---|---|---|---|---|---|---|
| tsc | default | 21047 | 6.7 | 6.9 | 86.1 | 3.1 | 3.9 | 59.5 | 7.2 | 14.6/15.2 |
| tsc | no-inline | 29246 | 8.4 | 8.8 | 83.4 | 3.9 | 3.9 | 60.3 | 9.1 | 14.6/15.2 |
| awfy-richards | default | 4674 | 0.1 | 99.3 | 0.0 | 0.6 | 0.1 | 96.6 | 0.1 | 18.6/79.0 |
| awfy-richards | no-inline | 4745 | 0.0 | 99.2 | 0.0 | 0.7 | 0.1 | 89.7 | 0.0 | 18.6/79.0 |
| awfy-deltablue | default | 4760 | 1.2 | 84.0 | 11.6 | 0.6 | 3.8 | 94.7 | 1.3 | 18.6/79.0 |
| awfy-deltablue | no-inline | 4802 | 10.2 | 64.4 | 31.0 | 2.3 | 2.3 | 91.5 | 10.7 | 18.6/79.0 |
| awfy-nbody | default | 4815 | 0.1 | 81.3 | 0.0 | 0.4 | 18.3 | 0.1 | 0.1 | 18.6/79.0 |
| awfy-nbody | no-inline | 5007 | 15.9 | 81.5 | 0.0 | 0.6 | 17.9 | 15.9 | 19.5 | 18.6/79.0 |
| vscode-diff | default | 4775 | 23.9 | 34.0 | 61.1 | 0.6 | 4.3 | 31.8 | 25.2 | 23.0/28.1 |
| vscode-diff | no-inline | 4814 | 39.1 | 43.4 | 53.9 | 0.8 | 1.9 | 44.4 | 40.1 | 23.0/28.1 |
| hono-fetch | default | 4797 | 0.2 | 0.2 | 20.1 | 70.8 | 8.9 | 14.3 | 0.8 | 22.0/22.2 |
| hono-fetch | no-inline | 4839 | 0.2 | 0.2 | 13.8 | 79.2 | 6.8 | 15.0 | 1.5 | 22.0/22.2 |
| hono-routers | default | 4780 | 6.2 | 6.2 | 69.7 | 16.5 | 7.6 | 6.2 | 8.2 | 22.0/22.2 |
| hono-routers | no-inline | 4779 | 6.0 | 6.0 | 74.7 | 11.4 | 7.9 | 6.0 | 7.4 | 22.0/22.2 |
| zod-parse | default | 4770 | 0.0 | 0.0 | 69.9 | 16.0 | 14.1 | 0.0 | 0.0 | 9.4/9.8 |
| zod-parse | no-inline | 4785 | 0.0 | 0.0 | 73.6 | 13.8 | 12.6 | 0.0 | 0.0 | 9.4/9.8 |
| datefns-fmtparse | default | 4757 | 5.9 | 6.1 | 74.8 | 13.2 | 5.9 | 9.7 | 7.1 | 13.7/15.1 |
| datefns-fmtparse | no-inline | 4765 | 11.0 | 11.2 | 77.3 | 6.0 | 5.5 | 15.4 | 12.3 | 13.7/15.1 |
