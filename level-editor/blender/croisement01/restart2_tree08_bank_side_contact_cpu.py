"""Distinguish true bank-side clearance from vertical-column root support."""
import hashlib
import json
from pathlib import Path
import numpy as np
from vtkmodules.vtkCommonCore import reference
from vtkmodules.vtkCommonDataModel import vtkStaticCellLocator
from restart2_tree08_root_embedding_cpu import mesh, R
from restart2_tree08_fork_kernel import poly

O=R/'tree08-bank-side-contact-cpu-v1'
O.mkdir(exist_ok=False)
receipt=json.loads((R/'tree08-wood-prototype-v14-root-ray/current-contact/receipt.json').read_text())
route=json.loads((R/'tree08-wood-prototype-v14-root-ray/current-contact/root-route-visibility.json').read_text())
receivers=[]
for binding in receipt['bindings']:
    vertices,faces=mesh(binding['asset']); vertices+=binding['translation']
    locator=vtkStaticCellLocator(); locator.SetDataSet(poly(vertices,faces)); locator.BuildLocator()
    receivers.append((binding['asset']['id'],locator,vertices,faces))
rows=[]
for sample in route['samples']:
    hit=np.array(sample['wood_hit']); candidates=[]
    for name,locator,vertices,faces in receivers:
        closest=[0.,0.,0.]; cell=reference(0); sub=reference(0); distance2=reference(0.)
        locator.FindClosestPoint(hit,closest,cell,sub,distance2)
        triangle=vertices[faces[int(cell)]]; normal=np.cross(triangle[1]-triangle[0],triangle[2]-triangle[0]); normal/=np.linalg.norm(normal)
        candidates.append(dict(receiver=name,closest=closest,distance=float(distance2)**.5,normal=normal.tolist(),face=int(cell)))
    rows.append(dict(native=sample['native'],front_point=sample['wood_hit'],nearest=min(candidates,key=lambda q:q['distance'])))
report=dict(status='DIAGNOSTIC_ONLY',reference_model_sha256=receipt['model_sha256'],rows=rows,max_front_distance=max(r['nearest']['distance'] for r in rows),scope='Source-facing root sample to actual bound receiver triangle, including vertical bank walls. This is not root-body contact: the front can stand off while an inferred back touches. Compare to prior vertical-column-only constraints before constructing support.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'samples':len(rows),'max_front_distance':report['max_front_distance'],'distance_quantiles':np.quantile([r['nearest']['distance'] for r in rows],[0,.5,.9,1]).tolist(),'near_vertical_bank':sum(abs(r['nearest']['normal'][2])<.1 for r in rows)}))
