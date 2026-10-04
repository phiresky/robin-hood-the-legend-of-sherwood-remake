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
    state=sys.argv[sys.argv.index('--state')+1] if '--state' in sys.argv else 'covered'
    assert state in ('covered','applied')
    source=OUT/'state-target-evidence/rock-trap'
    manifest=json.loads((source/'manifest.json').read_text());left,top,right,bottom=manifest['bbox']
    alpha=np.array(Image.open(source/('tick--01.png' if state=='covered' else 'tick-104.png')))[:,:,3]>0
    yy,xx=np.where(alpha);results=[]
    for path in [OUT/'rock-trap-state-candidate-v8',candidate]:
        bpy.ops.wm.open_mainfile(filepath=str(path/'worker.blend'));bpy.context.view_layer.update()
        verts=[];faces=[]
        for obj in bpy.data.objects:
            if obj.type!='MESH' or obj.get('state_endpoint')!=state:continue
            offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(i+offset for i in p.vertices)for p in obj.data.polygons)
        tree=BVHTree.FromPolygons(verts,faces);hits=[]
        for x,y in zip(xx,yy):hits.append(tree.ray_cast(point(float(left+x+.5),float(top+y+.5),0)+RAY*5000,-RAY)[0] is not None)
        results.append(dict(path=str(path),model_sha256=sha(path/'worker.blend'),hits=hits,covered=sum(hits),samples=len(hits),coverage=sum(hits)/len(hits)))
    before,after=[np.array(r.pop('hits'))for r in results]
    report=dict(status='coverage comparison; visual quality and support remain separate',models=results,newly_missing=int((before&~after).sum()),newly_covered=int((after&~before).sum()),source_manifest_sha256=sha(source/'manifest.json'))
    missing={(int(x),int(y))for x,y,hit in zip(xx,yy,after)if not hit};remaining=set(missing);components=[]
    while remaining:
        pending=[remaining.pop()];component=[]
        while pending:
            x,y=pending.pop();component.append([x,y])
            for dx,dy in ((-1,0),(1,0),(0,-1),(0,1)):
                neighbor=(x+dx,y+dy)
                if neighbor in remaining:remaining.remove(neighbor);pending.append(neighbor)
        components.append(dict(pixels=len(component),crop_coordinates=component))
    report['missing_components']=sorted(components,key=lambda r:-r['pixels'])
    rgba=np.array(Image.open(source/('tick--01.png'if state=='covered'else 'tick-104.png')).convert('RGBA'))
    rgba[yy[~after],xx[~after]]=[255,30,30,255]
    Image.fromarray(rgba).save(candidate/f'{state}-missing-native-pixels.png')
    report['state']=state
    (candidate/('native-coverage-comparison.json'if state=='covered'else 'applied-native-coverage-comparison.json')).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))


if __name__=='__main__':main()
