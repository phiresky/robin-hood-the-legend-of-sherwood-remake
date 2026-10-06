"""Read-only approved physical receiver context for tree25 residual source roles."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import restart6_source_gap_audit as audit
from render_slots import acquire,release
r=next(x for x in json.load(open(audit.OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json'))['records']if x['asset_id']=='croisement02-tree-25');audit.SPECS[25]=(Path(r['model']),r['model_sha256']);coords=next(x['coordinates']for x in json.load(open(audit.ROOT/'remaining-inventory-v1/report.json'))['rows']if x['mask']==25);mask=np.zeros((1152,1792),np.uint8)
for x,y in coords:mask[y,x]=255
Image.fromarray(mask).save(audit.ROOT/'source-audit-v1/exposed-25.png')
acquire()
try:audit.main(25)
finally:release()
