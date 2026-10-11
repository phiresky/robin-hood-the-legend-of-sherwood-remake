"""Compare private wood against the installed Tree32 crown and shrub71.

This is a scoped alpha-aware ownership check, not full-scene parity.
"""
import argparse,hashlib,json,sys,shutil
from pathlib import Path
import numpy as np
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from physical_opacity import OpacityRegistry
from continuous_wood_field import SIN,RAY
from physical_tree32_reviewed_domain_cpu import reviewed_source,BASE as DOMAIN
from audit_tree32_source_islands import read

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def installed(asset):
    path=ROOT/'level-editor/library/3d-assets/croisement02'/asset/'model.glb';descriptor=path.with_name('asset.json');digest=sha(path);description_sha=sha(descriptor);origin=Vector(json.loads(descriptor.read_text())['source_origin_scene'])
    bpy.ops.wm.read_factory_settings(use_empty=True);bpy.ops.import_scene.gltf(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.data.objects if o.type=='MESH']
    for obj in objects:
        world=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=world;obj.location+=origin
    bpy.context.view_layer.update();points=[];triangles=[];owners=[];opacity=OpacityRegistry()
    for obj in objects:
        obj.data.calc_loop_triangles();offset=len(points);points.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        for triangle in obj.data.loop_triangles:
            opacity.add(obj,obj.data,triangle);triangles.append(tuple(offset+i for i in triangle.vertices));owners.append(dict(asset=asset,source_node=obj.get('source_node'),component=obj.get('projection_component')))
    if not triangles:raise ValueError('Installed asset has no geometry')
    return dict(path=path,sha=digest,descriptor=descriptor,descriptor_sha=description_sha,points=points,triangles=triangles,owners=owners,opacity=opacity.records)
def compose(groups):
    points=[];triangles=[];owners=[];opacity=OpacityRegistry()
    for asset,scope in groups:
        offset=len(points);points.extend(asset['points']);selected=[i for i,o in enumerate(asset['owners'])if scope is None or ((o['component']=='crown')==(scope=='crown'))]
        triangles.extend(tuple(i+offset for i in asset['triangles'][j])for j in selected);owners.extend(asset['owners'][j]for j in selected);opacity.records.extend(asset['opacity'][j]for j in selected)
    return opacity.wrap(BVHTree.FromPolygons(points,triangles,all_triangles=True)),owners

def main():
    p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
    if a.output.exists():raise FileExistsError(a.output)
    observed,_=reviewed_source(32,(935,536,1253,759));excluded=json.loads((DOMAIN/'proposal.json').read_text())['changed_pixels'];acquire(slots=2)
    try:
        memory=int(next(line.split()[1]for line in Path('/proc/meminfo').read_text().splitlines()if line.startswith('MemAvailable:')))*1024
        if memory<6*1024**3 or shutil.disk_usage(ROOT).free<10*1024**3:raise RuntimeError('Resource guard stopped neighborhood audit')
        tree=installed('croisement02-tree-32');shrub=installed('croisement02-shrub-71');candidate=read(a.candidate.resolve(),'croisement02-tree-32');states={'installed':compose([(tree,None),(shrub,None)]),'candidate':compose([(tree,'crown'),(candidate,'wood'),(shrub,None)])}
        def owner(state,x,y,dx=.5,dy=.5):
            bvh,owners=states[state];point=Vector((x+dx,-(y+dy)/SIN,0))+Vector(RAY)*5000;hit,_,index,_=bvh.ray_cast(point,-Vector(RAY));record=owners[index]if hit is not None else None
            return {k:record.get(k)for k in ['asset','source_node','component']}if record else None
        samples=[];introduced=[]
        for x,y in excluded:
            for dx,dy in [(.5,.5),(.25,.25),(.75,.25),(.25,.75),(.75,.75)]:
                old=owner('installed',x,y,dx,dy);new=owner('candidate',x,y,dx,dy);row=dict(pixel=[x,y],offset=[dx,dy],installed=old,candidate=new);samples.append(row)
                if old!=new:introduced.append(row)
        source_regressions=[];counts={'installed':0,'candidate':0}
        for y,x in np.argwhere(observed):
            x,y=int(x)+935,int(y)+536;hits={state:owner(state,x,y)for state in states};valid={state:(hit is not None and hit['asset']=='croisement02-tree-32' and hit['component']!='crown')for state,hit in hits.items()}
            for state in states:counts[state]+=int(valid[state])
            if valid['installed'] and not valid['candidate']:source_regressions.append(dict(pixel=[x,y],**hits))
        for asset in [tree,shrub,candidate]:
            if sha(asset['path'])!=asset['sha']:raise ValueError('Read-only asset changed')
            if 'descriptor' in asset and sha(asset['descriptor'])!=asset['descriptor_sha']:raise ValueError('Installed placement changed')
        report=dict(status='PASS_SCOPED_OWNERSHIP_COMPARISON'if not introduced and not source_regressions else'HOLD',candidate=str(a.candidate.resolve()),candidate_sha256=candidate['sha'],installed_inputs=[dict(path=str(t['path']),sha256=t['sha'],descriptor_sha256=t['descriptor_sha'])for t in [tree,shrub]],excluded_source_pixels=excluded,excluded_pixel_samples=samples,changed_excluded_pixel_owners=introduced,observed_wood_first_hit_counts=counts,observed_source_centers=int(observed.sum()),introduced_wood_source_occlusions=source_regressions,limitations=['Only Tree32 and shrub71 are included; other neighbors and full-scene parity remain separate.','Compares physical alpha-aware first-hit ownership, not RGB equality or approval.'])
        a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k]for k in ['status','observed_wood_first_hit_counts','observed_source_centers']},indent=2));print('Changed excluded samples:',len(introduced),'Source regressions:',len(source_regressions))
    finally:release()
if __name__=='__main__':main()
