"""Replace a mirrored inferred twig with own-native leaf texture, preserving geometry."""
import json
import shutil
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from central_support_geometry import leaf_signature


def main():
    old=OUT/'understory-candidates/native-80-scoped-v1';new=OUT/'understory-candidates/native-80-leaf-fill-v2'
    new.mkdir(exist_ok=False);folder=new/'shrub-80';shutil.copytree(old/'shrub-80',folder)
    for name in ('catalog.json','previous-catalog.json','mask-inventory.json','source-masks.json','worker-source-masks.json','worker-catalog.json','grouping-review.json','worker-grouping-review.json','source-rgb-validation.json'):
        text=(old/name).read_text().replace(str(old),str(new));(new/name).write_text(text)
    shutil.copytree(old/'inventory',new/'inventory');shutil.copytree(old/'worker-inventory',new/'worker-inventory')
    shutil.copy2(old/'domain-491.png',new/'domain-491.png')
    packet=json.loads((folder/'partition.json').read_text());packet['directory']=str(folder)
    packet['inferred_material_correction']='West RGB uses small own-native green leaf patches; prior mirrored recognizable twig removed; alpha/geometry unchanged'
    write_json(folder/'partition.json',packet)
    observed=np.asarray(Image.open(folder/'observed-source.png').convert('RGBA'));rgb=observed[:,:,:3].astype(float)
    green=(observed[:,:,3]>127)&(rgb[:,:,1]>rgb[:,:,0]*1.04)&(rgb[:,:,1]>rgb[:,:,2]*1.35)
    patches=[]
    for y in range(observed.shape[0]-3):
        for x in range(35,observed.shape[1]-3):
            if green[y:y+3,x:x+3].all():patches.append(observed[y:y+3,x:x+3,:3].copy())
    if len(patches)<8:raise ValueError('Insufficient own-native leaf-only patches')
    rng=np.random.default_rng(800802);fill=np.zeros((observed.shape[0],35,3),dtype='uint8')
    for y in range(0,len(fill),3):
        for x in range(0,35,3):
            tile=patches[int(rng.integers(len(patches)))];h,w=min(3,len(fill)-y),min(3,35-x);fill[y:y+h,x:x+w]=tile[:h,:w]
    changed=[]
    for name in ('complete-source.png','inferred-source.png','unknown-front.png'):
        before=np.asarray(Image.open(folder/name).convert('RGBA'));after=before.copy();after[:,:35,:3]=fill
        if not np.array_equal(before[:,:,3],after[:,:,3]) or not np.array_equal(before[:,35:],after[:,35:]):raise ValueError('Native region or alpha changed')
        Image.fromarray(after).save(folder/name);changed.append(dict(image=name,old_sha256=sha(old/'shrub-80'/name),new_sha256=sha(folder/name),changed_rgb_pixels=int(np.any(before[:,:,:3]!=after[:,:,:3],axis=2).sum())))
    bpy.ops.wm.open_mainfile(filepath=str(old/'input.blend'));bpy.context.preferences.filepaths.save_version=0
    obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='foliage-shrub-080')
    counts=(len(obj.data.vertices),len(obj.data.polygons),len(obj.data.loops));before=leaf_signature(obj.data,*counts)
    images={}
    for mat in obj.data.materials:
        for node in mat.node_tree.nodes:
            if node.type!='TEX_IMAGE' or node.image is None:continue
            path=Path(bpy.path.abspath(node.image.filepath))
            if path.parent!=old/'shrub-80':continue
            if path.name not in images:
                images[path.name]=bpy.data.images.load(str(folder/path.name),check_existing=False);images[path.name].pack()
            node.image=images[path.name]
    if not {'inferred-source.png','unknown-front.png'}<=set(images):raise ValueError('Corrected material images were not bound')
    if before!=leaf_signature(obj.data,*counts):raise ValueError('Appearance correction changed mesh')
    bpy.ops.wm.save_as_mainfile(filepath=str(new/'input.blend'),compress=True)
    source=OUT/'animation-references/composite-frame-0.png';worker=new/'assets/croisement02-shrub-80'
    write_json(new/'scope-derivation.json',dict(source=str(old/'input.blend'),source_sha256=sha(old/'input.blend'),full_catalog_sha256=sha(new/'catalog.json'),full_inventory_sha256=sha(new/'inventory/inventory.json'),targets=[worker.name],reason='Appearance-only native west completion; mesh/UV/ownership unchanged; no prior approval inherited'))
    for label,inventory in [('grouping-review.json','inventory/inventory.json'),('worker-grouping-review.json','worker-inventory/inventory.json')]:
        catalog='worker-catalog.json' if label.startswith('worker-') else 'catalog.json'
        write_json(new/label,dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(new/catalog),inventory_sha256=sha(new/inventory),evidence='Existing isolated geometry unchanged; only inferred west source RGB corrected from own leaf patches'))
    prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=new/'worker-catalog.json',inventory_path=new/'worker-inventory/inventory.json',review_path=new/'worker-grouping-review.json',source_mask_manifest=new/'worker-source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    saved=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='foliage-shrub-080')
    if before!=leaf_signature(saved.data,*counts):raise ValueError('Saved appearance worker changed geometry or UVs')
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=json.loads((old/'assets'/worker.name/'inspection/refinement.json').read_text());report['source_packet']=str(folder/'partition.json');report['model_sha256']=sha(worker/'model.blend');report['crown']['inferred_material_correction']=packet['inferred_material_correction']
    report['limitations'].append('West continuation alpha and shape are inferred; recognizable mirrored twig replaced by own small green leaf patches')
    write_json(inspection/'refinement.json',report)
    write_json(inspection/'appearance-preservation.json',dict(model_sha256=sha(worker/'model.blend'),prior_input=str(old/'input.blend'),prior_input_sha256=sha(old/'input.blend'),mesh_signature=before,geometry_uv_ownership_unchanged=True,all_image_alpha_unchanged=True,native_in_map_rgba_unchanged=True,leaf_patch_size=[3,3],native_leaf_patch_count=len(patches),changed=changed,user_approval=None))
    audit(worker);render_workspace(worker,384,release_slot=False)


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
