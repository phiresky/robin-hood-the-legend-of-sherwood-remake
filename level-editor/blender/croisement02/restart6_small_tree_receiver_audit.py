"""Read-only receiver checks for the remaining seven probable wood edge centers."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import restart6_source_gap_audit as audit
from render_slots import acquire,release
rows=json.load(open(audit.OUT/'restart2-textures/batch10-linked-static-v1/source-pins.json'))['receivers'].values();inventory=json.load(open(audit.ROOT/'remaining-inventory-v1/report.json'))['rows']
for index in [24,38]:
 row=next(r for r in rows if r['asset_group']==f'croisement02-tree-{index}');audit.SPECS[index]=(Path(row['model']),row['model_sha256']);entry=next(r for r in inventory if r['mask']==index);mask=np.zeros((1152,1792),dtype=np.uint8)
 for x,y in entry['coordinates']:mask[y,x]=255
 path=audit.ROOT/f'source-audit-v1/exposed-{index}.png';assert not path.exists();Image.fromarray(mask).save(path)
 acquire()
 try:audit.main(index)
 finally:release()
