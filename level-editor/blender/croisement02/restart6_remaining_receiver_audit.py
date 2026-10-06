"""Inspect exact approved receivers for the remaining source gap clusters."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import restart6_source_gap_audit as audit
from render_slots import acquire,release
index=int(sys.argv[sys.argv.index('--')+1]);assert index in [18,19,25]
rows=json.loads((audit.OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json').read_text())['records'];row=next(r for r in rows if r['asset_id']==f'croisement02-tree-{index:02d}');audit.SPECS[index]=(Path(row['model']),row['model_sha256'])
mask=np.zeros((1152,1792),dtype=np.uint8);entry=next(r for r in json.loads((audit.ROOT/'remaining-inventory-v1/report.json').read_text())['rows']if r['mask']==index)
for x,y in entry['coordinates']:mask[y,x]=255
path=audit.ROOT/f'source-audit-v1/exposed-{index}.png'
if not path.exists():Image.fromarray(mask).save(path)
acquire()
try:audit.main(index)
finally:release()
