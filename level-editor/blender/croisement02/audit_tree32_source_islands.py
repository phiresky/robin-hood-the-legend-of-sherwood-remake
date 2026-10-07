"""Read-only, alpha-aware nine-pixel ownership proof in the local neighbourhood."""
import argparse,hashlib,json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from physical_opacity import OpacityRegistry
from wood_source_partition import RAY,SIN
OUT=ROOT/'level-editor/work/croisement02-refinement'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()

def read(path,asset):
    digest=sha(path);bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
    opacity=OpacityRegistry();points=[];triangles=[];owners=[]
    for obj in bpy.data.collections['Croisement02 Working'].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
        obj.data.calc_loop_triangles();offset=len(points)
        points.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        for triangle in obj.data.loop_triangles:
            opacity.add(obj,obj.data,triangle);triangles.append(tuple(offset+i for i in triangle.vertices));owners.append(dict(object=obj.name,asset=asset,source_node=obj.get('source_node'),component=obj.get('projection_component')))
    if not triangles:raise ValueError('No exact neighbourhood asset found')
    # Records own their UVs, world points and copied alpha arrays; they do not
    # rely on Blender image datablocks surviving the next read-only file load.
    return dict(path=path,sha=digest,points=points,triangles=triangles,owners=owners,opacity=opacity.records)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists():raise FileExistsError(args.output)
    packet=OUT/'wood-sweep-cpu-review/integration-packet-v1/recipe.json'
    pixels=next(r for r in json.loads(packet.read_text())['records'] if r['tree']==32)['unresolved_source_coordinates']
    paths={'prior':OUT/'root-stem-round-3/assets/croisement02-tree-32/model.blend','current':OUT/'restart2-wood/combined-crown-v2/assets/croisement02-tree-32/model.blend','shrub':OUT/'understory-candidates/native-71-rooted-v2/assets/croisement02-shrub-71/model.blend'}
    acquire()
    try:
        assets={name:read(path,'croisement02-shrub-71' if name=='shrub' else 'croisement02-tree-32') for name,path in paths.items()};records=[]
        for state in ['prior','current']:
            points=[];triangles=[];owners=[];opacity=OpacityRegistry()
            for asset in [assets[state],assets['shrub']]:
                offset=len(points);points.extend(asset['points']);triangles.extend(tuple(i+offset for i in t) for t in asset['triangles']);owners.extend(asset['owners']);opacity.records.extend(asset['opacity'])
            tree=opacity.wrap(BVHTree.FromPolygons(points,triangles,all_triangles=True))
            for index,(x,y) in enumerate(pixels):
                samples=[]
                for dx,dy in [(.5,.5),(.25,.25),(.75,.25),(.25,.75),(.75,.75)]:
                    origin=Vector((x+dx,-(y+dy)/SIN,0))+Vector(RAY)*5000
                    hit,normal,triangle,distance=tree.ray_cast(origin,-Vector(RAY))
                    samples.append(dict(offset=[dx,dy],owner=owners[triangle] if hit is not None else None,world=list(hit) if hit is not None else None))
                records.append(dict(state=state,index=index,pixel=[x,y],samples=samples))
        for asset in assets.values():
            if sha(asset['path'])!=asset['sha']:raise ValueError('Read-only audit changed input')
        report=dict(status='Scoped alpha-aware first-hit evidence; independent source-role review required',scope='Tree32 plus shrub71 only, prior and current tree variants. This is not a full-scene parity claim.',inputs=[dict(path=str(a['path']),sha256=a['sha']) for a in assets.values()],pixels=records,model_saved=False,rendered=False)
        args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
    finally:release()

if __name__=='__main__':main()
