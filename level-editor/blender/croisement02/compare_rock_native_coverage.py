"""Compare actual covered-rock ray coverage before and after contour refinement."""
import json, sys
from pathlib import Path
import bpy
import numpy as np
from mathutils.bvhtree import BVHTree
from PIL import Image
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point,sha


def main():
    candidate=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve()
    source=OUT/'state-target-evidence/rock-trap'
    manifest=json.loads((source/'manifest.json').read_text());left,top,right,bottom=manifest['bbox']
    alpha=np.array(Image.open(source/'tick--01.png'))[:,:,3]>0
    yy,xx=np.where(alpha);results=[]
    for path in [OUT/'rock-trap-state-candidate-v8',candidate]:
        bpy.ops.wm.open_mainfile(filepath=str(path/'worker.blend'));bpy.context.view_layer.update()
        verts=[];faces=[]
        for obj in bpy.data.objects:
            if obj.type!='MESH' or obj.get('state_endpoint')!='covered':continue
            offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(i+offset for i in p.vertices)for p in obj.data.polygons)
        tree=BVHTree.FromPolygons(verts,faces);hits=[]
        for x,y in zip(xx,yy):hits.append(tree.ray_cast(point(float(left+x+.5),float(top+y+.5),0)+RAY*5000,-RAY)[0] is not None)
        results.append(dict(path=str(path),model_sha256=sha(path/'worker.blend'),hits=hits,covered=sum(hits),samples=len(hits),coverage=sum(hits)/len(hits)))
    before,after=[np.array(r.pop('hits'))for r in results]
    report=dict(status='coverage comparison; visual quality and support remain separate',models=results,newly_missing=int((before&~after).sum()),newly_covered=int((after&~before).sum()),source_manifest_sha256=sha(source/'manifest.json'))
    (candidate/'native-coverage-comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))


if __name__=='__main__':main()
