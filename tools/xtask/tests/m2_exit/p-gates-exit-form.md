# P-gates P1, P2 and P4: the exit form (fixture)

The exit form of plan v14 §4.1 (3): the first table with the columns Gate and Result (the form of `docs/measurements/2026-11-16-p-gates-exit-form.md`).

## Exit form

| Gate | Result | Trigger (01 §7) | Reading at `0123456789abcdef0123456789abcdef01234567` | Evidence |
|---|---|---|---|---|
| P1 | not tripped | Eval context or the depth limit is neither upstream nor in a local set of 3 or fewer patches | both on their tested workarounds; 0 of 3 local oxc patches | the oxc_parser sources |
| P2 | not tripped | Measured work for oxc version bumps is more than 1 PM per year (est.) | 2 bumps, 0.59 and 1.73 hours | the P2 table below |
| P4 | not tripped | Any test262 lone-surrogate eval case fails through the shim | 0 of the 11 baseline files fail | the test262 artifacts |

## P2: hours per oxc bump

| Run | Date | Move | Hours | Source |
|---|---|---|---|---|
| DEPS-BUMP-1 | 2026-11-10 | oxc 0.157.0 to 0.158.0 | 1.73 | the orchestrator log |
