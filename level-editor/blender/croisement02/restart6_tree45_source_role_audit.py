"""Read-only audit of the sixteen contiguous own-source trunk edge centers."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import restart6_source_gap_audit as audit
from render_slots import acquire,release
from evidence_io import write_json,sha
rows=json.load(open(audit.OUT/'restart2-textures/batch10-linked-static-v1/source-pins.json'))['receivers'].values();row=next(r for r in rows if r['asset_group']=='croisement02-tree-45');audit.SPECS[45]=(Path(row['model']),row['model_sha256']);entry=next(r for r in json.load(open(audit.ROOT/'remaining-inventory-v1/report.json'))['rows']if r['mask']==45);confirmed=[p for p in entry['coordinates']if p!=[1359,982]];assert len(confirmed)==16;mask=np.zeros((1152,1792),dtype=np.uint8)
for x,y in confirmed:mask[y,x]=255
path=audit.ROOT/'source-audit-v1/exposed-45.png';assert not path.exists();Image.fromarray(mask).save(path);write_json(audit.ROOT/'tree45-role-proposal-v1.json',dict(confirmed_contiguous_trunk_edge=confirmed,uncertain_context=[[1359,982]],uncertain_context_also_in_mask=98,source_native_sha256=sha(audit.OUT/'animation-references/composite-frame-0.png'),scope='Source visual role proposal; no geometry or ownership transfer.'))
acquire()
try:audit.main(45)
finally:release()
