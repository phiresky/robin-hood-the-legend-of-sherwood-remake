"""Audit all four authored poses against unchanged alpha-aware static receivers."""
from pathlib import Path
here=Path(__file__).resolve().parent
source=(here/'contact_conformed.py').read_text()
replacements={"'geometry-v3-contacts'":"'geometry-v4-all-pose-contacts'","conformation-v2/depth-field.json":"conformation-v4-cell-bounds/depth-field.json","conformed-selected.json":"conformed-v4-selected.json","for phase,f in [('initial',row['art']['states']['initial']['frames'][0]),('applied',row['art']['states']['transition']['frames'][-1])]:":"for phase,f in zip(['initial','transition-0','transition-1','applied'], row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']):","Endpoint volumes only; three native transition poses and continuous motion not tested.":"All four discrete native poses. No interpolated motion is asserted; source animation holds separate poses."}
for old,new in replacements.items():
 assert old in source,old
 source=source.replace(old,new)
exec(compile(source,str(here/'contact_conformed.py'),'exec'),{'__name__':__name__,'__file__':str(here/'contact_conformed.py')})
