"""Attribute native first-hit differences to observed versus inferred materials."""
import sys,json
from pathlib import Path
import numpy as np
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_check_sign_bend_sources import sample
from catalog import OUT
from evidence_io import write_json,sha
from render_slots import acquire,release

def main():
    source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';candidate=OUT/'restart2-fence/shrub57-sign-bend-v5/model.blend';box=[-42,200,116,390]
    old,roles,_=sample(source,box);new,newroles,_=sample(candidate,box);different=np.any(abs(old-new)>1e-5,axis=2)
    rows=[]
    for y,x in zip(*np.nonzero(different&(roles==1))):rows.append(dict(pixel=[int(x+box[0]),int(y+box[1])],old_rgba=old[y,x].tolist(),new_rgba=new[y,x].tolist(),new_role=int(newroles[y,x])))
    dest=OUT/'restart2-fence/shrub57-sign-bend-v5/source-changes.json';assert not dest.exists();write_json(dest,dict(model_sha256=sha(candidate),source_model_sha256=sha(source),observed_changes=rows,role_counts={str(i):sum(r['new_role']==i for r in rows) for i in [0,1,2]}));print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
