"""Audit all four authored poses against unchanged alpha-aware static receivers."""
from pathlib import Path
here=Path(__file__).resolve().parent
source=(here/'contact_conformed.py').read_text()
replacements={"'geometry-v3-contacts'":"'geometry-v6-all-pose-contacts'","conformation-v2/depth-field.json":"conformation-v5-local-cell-bounds/depth-field.json","conformed-selected.json":"conformed-v5-selected.json","for phase,f in [('initial',row['art']['states']['initial']['frames'][0]),('applied',row['art']['states']['transition']['frames'][-1])]:":"for phase,f in zip(['initial','transition-0','transition-1','applied'], row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']):","Endpoint volumes only; three native transition poses and continuous motion not tested.":"All four discrete native poses. No interpolated motion is asserted; source animation holds separate poses."}
replacements['triangle=int(ti),source_cell'] = 'triangle=int(ti),primitive_index=int(pi),node_index=int(ni),source_cell'
replacements['from restart14_butterfly07_prism_contacts import alpha_maximum'] = 'from restart14_butterfly07_prism_contacts import alpha_maximum\nfrom restart406_alpha_contact_predicate import threshold_contact'
replacements['maximum,witness,cells=alpha_maximum(clipped,image,sampler,factor)'] = "decision=threshold_contact(clipped,image,sampler,factor,max(1e-12,cut-1e-8));maximum=decision.get('value',decision.get('upper_bound',0.));witness=decision['witness'];cells=decision['cells']"
replacements['if maximum+1e-8>=cut:'] = "if decision['intersects']:"
replacements['alpha_maximum=maximum,camera_space_witness'] = "alpha_witness_value=maximum,alpha_decision_mode=decision['mode'],camera_space_witness"
replacements["rounded02_models=q.pin(q.OUT/'rounded02-selected.json'),records=records"] = "rounded02_models=q.pin(q.OUT/'rounded02-selected.json'),alpha_predicate=q.pin(HERE.parent/'restart406_alpha_contact_predicate.py'),records=records"
for old,new in replacements.items():
 assert old in source,old
 source=source.replace(old,new)
exec(compile(source,str(here/'contact_conformed.py'),'exec'),{'__name__':__name__,'__file__':str(here/'contact_conformed.py')})
