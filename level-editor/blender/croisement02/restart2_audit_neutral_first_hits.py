"""Classify bounded neutral-render components against a frozen physical scene."""
import argparse,json,sys
from pathlib import Path
from collections import Counter
import bpy,numpy as np
from PIL import Image,ImageDraw
from scipy import ndimage
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import _tree
from tree_geometry import SIN,RAY
from audit_scene_first_hit import full_mask


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--stage',type=Path,default=OUT/'restart2-textures/whole126-scene-v2');parser.add_argument('--output',type=Path,default=OUT/'restart2-vegetation/neutral-first-hit-v1');parser.add_argument('--ground-known',type=Path,default=OUT/'ground-source75-restoration-v3/ground-observed-domain.png');parser.add_argument('--comparison-native',type=Path)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else [])
    stage=args.stage.resolve();review=OUT/'restart2-textures/whole126-review-v1'
    output=args.output.resolve();output.mkdir(exist_ok=False)
    assembly=json.loads((stage/'assembly.json').read_text());scene_hash=sha(stage/'scene.blend');assert scene_hash==assembly['model_sha256']
    diagnostics=json.loads((review/'neutral-components.json').read_text());assert sha(review/'native.png')==diagnostics['native_sha256']
    rgba=np.asarray(Image.open(review/'native.png').convert('RGBA'));components=sorted(diagnostics['components'],key=lambda r:-r['pixels'])[:20]
    chosen=[]
    for row in components:
        value=row['neutral_value'];mask=np.all(rgba[:,:,:3]==value,axis=2)
        found=None
        for structure in [None,np.ones((3,3),bool)]:
            labels,count=ndimage.label(mask,structure);slices=ndimage.find_objects(labels)
            for n,sl in enumerate(slices,1):
                if sl is None:continue
                y,x=sl;box=[x.start,y.start,x.stop,y.stop]
                if box==row['bounds'] and int((labels[sl]==n).sum())==row['pixels']:
                    yy,xx=np.nonzero(labels[sl]==n);found=(yy+y.start,xx+x.start);break
            if found is not None:break
        if found is None:raise ValueError('Component reconstruction failed '+str(row))
        chosen.append((row,*found))
    bpy.ops.wm.open_mainfile(filepath=str(stage/'scene.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not o.hide_render]
    tree,owners,_=_tree(objects)
    materials=[];depsgraph=bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        evaluated=obj.evaluated_get(depsgraph);mesh=evaluated.to_mesh();mesh.calc_loop_triangles()
        materials.extend(mesh.materials[t.material_index].name if t.material_index<len(mesh.materials) and mesh.materials[t.material_index] else '<none>'for t in mesh.loop_triangles);evaluated.to_mesh_clear()
    assert len(materials)==len(owners)
    authority_path=Path(assembly['selection']).parent/'worker-source-authorities.json';authorities=json.loads(authority_path.read_text());cache={};permissions=[]
    for source in authorities['source_records']:
        record=source.get('mask_inventory');rules=source.get('own_assignments',{}).get('exterior',[])
        if not record or not rules:continue
        path=Path(record['path']);assert sha(path)==record['sha256'];inventory=json.loads(path.read_text());mapping={r['index']:r for r in inventory['masks']}
        permitted=np.zeros((1152,1792),bool)
        for rule in rules:
            def get(index):
                key=(str(path),index)
                if key not in cache:cache[key]=full_mask(mapping[index],path)
                return cache[key]
            allow=np.zeros_like(permitted)
            for index in rule.get('mask_indices',[]):allow|=get(index)
            for index in rule.get('exclude_mask_indices',[]):allow&=~get(index)
            permitted|=allow
        permissions.append((source['asset_id'],permitted))
        cache.clear()
    ground_path=args.ground_known.resolve();known=np.asarray(Image.open(ground_path).convert('L'))>0
    comparison=args.comparison_native or review/'native.png';compared=np.asarray(Image.open(comparison).convert('RGBA'));records=[];allpoints=[];annotated=Image.open(comparison).convert('RGB');draw=ImageDraw.Draw(annotated)
    for number,(row,yy,xx)in enumerate(chosen,1):
        counts=Counter();mats=Counter();samples=[]
        for y,x in zip(yy,xx):
            origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*6000
            hit,normal,index,distance=tree.ray_cast(origin,-RAY)
            if hit is None:asset='<no physical hit>';name=material=None
            else:
                obj=owners[index];asset=obj.get('asset_group')or obj.get('source_node')or obj.name;name=obj.name;material=materials[index]
            counts[asset]+=1
            if material:mats[(asset,material)]+=1
            samples.append(dict(pixel=[int(x),int(y)],asset=asset,object=name,material=material,hit=list(hit)if hit else None,ground_known=bool(known[y,x]),comparison_rgba=compared[y,x].tolist()))
        domain_counts={asset:int(mask[yy,xx].sum())for asset,mask in permissions if mask[yy,xx].any()}
        classification='mixed first-hit receivers; inspect counts'
        if len(counts)==1:
            asset=next(iter(counts))
            classification=('ground receiver visible in excluded/unknown source region' if 'ground-receiver' in asset and not known[yy,xx].any() else 'ground receiver within known domain' if 'ground-receiver' in asset else 'visible asset face; inspect its material/source coverage' if asset!='<no physical hit>' else 'no physical geometry')
        records.append(dict(component=number,**row,classification=classification,first_hits=dict(counts),ground_known_pixels=int(known[yy,xx].sum()),permitted_worker_domains=domain_counts,materials=[dict(asset=a,material=m,pixels=c)for(a,m),c in mats.most_common()],samples=samples))
        x0,y0,x1,y1=row['bounds'];draw.rectangle((x0,y0,x1-1,y1-1),outline='red',width=2);draw.text((x0,y0),str(number),fill='white')
        print('COMPONENT',number,row['pixels'],dict(counts),flush=True)
    annotated.save(output/'components.png')
    assert sha(stage/'scene.blend')==scene_hash
    write_json(output/'audit.json',dict(status='Read-only physical classification; not remediation authority',scene=str(stage/'scene.blend'),scene_sha256=scene_hash,native_sha256=diagnostics['native_sha256'],comparison_native=str(comparison),comparison_native_sha256=sha(comparison),baseline_components_from=str(review/'neutral-components.json'),assembly_sha256=sha(stage/'assembly.json'),worker_authorities_sha256=sha(authority_path),ground_known_sha256=sha(ground_path),method='Exact source pixel centers for largest20 equal-RGB connected components; shared alpha-aware one-sided BVH; each pixel records first-hit object/material and frozen worker permissions.',components=records,limitations=['Neutral RGB only selects diagnostic pixels, never assigns source ownership.','Components are fixed baseline126 pixel sets; the exact inspected scene and optional newer rendered colors are separately hash-bound.','Ground exposed inside a worker source domain indicates a projection/physical silhouette gap; it does not authorize ground infill.','Worker permissions can overlap and are metadata, not physical depth evidence.','Small boundary samples can disagree with multisample raster coverage.']))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
