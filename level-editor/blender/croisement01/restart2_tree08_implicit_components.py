"""Measure independent implicit components against actual native source ownership."""
import json,sys
from pathlib import Path
import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from PIL import Image
from vtkmodules.vtkCommonCore import reference
from vtkmodules.vtkCommonDataModel import vtkGenericCell,vtkStaticCellLocator
from restart2_tree08_fork_kernel import poly
R=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2';out=R/sys.argv[1];assert out.resolve().parent==R.resolve()
a=np.load(out/'candidate.npz');v,f=a['vertices'],a['faces'];edges=np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]);graph=coo_matrix((np.ones(len(edges)),(edges[:,0],edges[:,1])),shape=(len(v),len(v))).tocsr();n,labels=connected_components(graph,directed=False);flabel=labels[f[:,0]]
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);down=np.array([0.,-s,-c]);counts=np.zeros(n,int);lost=0
loc=vtkStaticCellLocator();loc.SetDataSet(poly(v,f));loc.BuildLocator();cell=vtkGenericCell()
for y,x in np.argwhere(np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0):
 p=np.array([x+331.5,0,0])+down*(y+11.5);t,sub,face=reference(0.),reference(0),reference(0)
 found=loc.IntersectWithLine(p+ray*2000,p-ray*2000,1e-9,t,[0.,0.,0.],[0.,0.,0.],sub,face,cell)
 if found:counts[flabel[int(face)]]+=1
 else:lost+=1
components=[]
for i in range(n):
 vv=v[labels==i];components.append(dict(id=i,vertices=len(vv),faces=int((flabel==i).sum()),source_pixels=int(counts[i]),min=vv.min(0).tolist(),max=vv.max(0).tolist()))
report=dict(components=components,lost_pixels=lost);(out/'components-source.json').write_text(json.dumps(report,indent=2)+'\n');np.savez_compressed(out/'component-labels.npz',vertices=labels,faces=flabel);print(json.dumps(report,indent=2),flush=True)
