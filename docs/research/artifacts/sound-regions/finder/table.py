import json
cols=['classes_total','classes_qualifying','classes_qualifying_pct','classes_qualifying_guarded_casts','classes_qualifying_guarded_casts_r4_lenient','classes_qualifying_v3_plus_generic_T_ok','classes_qualifying_open_world','member_access_total','member_access_dynamic','member_access_class_receiver','member_access_qualifying_receiver','member_access_qualifying_pct_of_dynamic','member_access_qualifying_guarded_casts_r4_lenient','member_access_qualifying_v3_plus_generic_T_ok','functions_total','functions_qualifying_t2','functions_qualifying_t2_pct','functions_qualifying_t0_strict','functions_qualifying_nontrivial','functions_qualifying_guarded_casts','functions_qualifying_guarded_casts_r4_lenient','functions_qualifying_v3_plus_generic_T_ok','boundary_call_sites','boundary_value_refs','boundary_escaping_closures','internal_call_sites','qualifying_functions_without_boundary','qualifying_functions_exported']
repos=['awfy','typescript','hono','vscode','zod','datefns','pixijs','preact']
S={r:json.load(open(f'out/{r}.summary.json')) for r in repos}
print('%-50s'%'metric'+''.join('%11s'%r for r in repos))
for c in cols: print('%-50s'%c+''.join('%11s'%S[r][c] for r in repos))
for r in repos: print(r, 'rules', S[r]['class_rule_counts'], 'sole', S[r]['class_sole_rule_counts'])
for r in repos: print(r, 'loo', S[r]['classes_qualifying_if_rule_off'])
for r in repos: print(r, 'sub', S[r]['class_sub_rule_counts'])
for r in repos: print(r, 'ffail', S[r]['function_fail_reasons'], 'sole', S[r]['function_sole_fail_reasons'])
for r in repos: print(r, 'fields', S[r]['field_kinds_qualifying'])
