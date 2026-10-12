"""Replace the northeast oak's stump/pole with the native branching silhouette.

The front silhouette is measured from native mask 11. Rounded branch sections,
rear depth and depth variation are inferred. The existing two foliage meshes
and all other assets remain unchanged. This produces a new model-review worker.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
EDITOR = HERE.parents[1]
ROOT = EDITOR/'work/needs-review/sherwood-refinement/northeast-oak-review'
NATIVE = EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.d/masks'
TARGETS = ['Tree 042 - tapered trunk.001',
           'Sherwood - Arbre03 tree 042 - limbs forks and terminal twigs.001', 'building-043.002']


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prepare():
    from scipy import ndimage
    ROOT.mkdir(parents=True, exist_ok=True)
    record = next(r for r in json.loads((NATIVE/'manifest.json').read_text())['masks'] if r['index']==11)
    a = np.asarray(Image.open(NATIVE/record['png']).convert('L')) > 0
    labels, _ = ndimage.label(a)
    counts = np.bincount(labels.ravel()); counts[0] = 0
    silhouette = labels == int(counts.argmax())
    retained = int(silhouette.sum())
    # Pixel-corner contacts otherwise create four faces on a vertical edge.
    # Bridge those ambiguous 2x2 corners with one inferred pixel. The source
    # texture still uses the unchanged native mask, so it cannot invent RGB.
    while True:
        tl,tr,bl,br = silhouette[:-1,:-1],silhouette[:-1,1:],silhouette[1:,:-1],silhouette[1:,1:]
        diagonal_a = np.argwhere(tl & br & ~tr & ~bl)
        diagonal_b = np.argwhere(tr & bl & ~tl & ~br)
        if not len(diagonal_a) and not len(diagonal_b):break
        for y,x in diagonal_a:silhouette[y,x+1]=True
        for y,x in diagonal_b:silhouette[y,x]=True
    bridged = int(silhouette.sum())-retained
    distance = ndimage.distance_transform_edt(silhouette)
    radius = ndimage.maximum_filter(distance, size=17)
    height, width = a.shape
    # Shared corner values keep the three gameplay-part partitions coincident.
    d = np.zeros((height+1,width+1)); r = np.zeros_like(d); active = np.zeros_like(d,bool)
    for oy,ox in [(0,0),(0,1),(1,0),(1,1)]:
        d[oy:oy+height,ox:ox+width] += distance/4
        r[oy:oy+height,ox:ox+width] = np.maximum(r[oy:oy+height,ox:ox+width],radius)
        active[oy:oy+height,ox:ox+width] |= silhouette
    thickness = np.sqrt(np.maximum(.25,d*(2*np.maximum(d,r)-d)))
    iy,ix = np.where(active); lookup = np.full(active.shape,-1,int);lookup[iy,ix]=np.arange(len(ix))
    x,y = ix+record['box_top_left'][0],iy+record['box_top_left'][1]
    sin,cos = math.sin(math.radians(35)),math.cos(math.radians(35))
    ground_y = 238.0
    base = np.column_stack((x,np.full(len(x),-ground_y/sin),(ground_y-y)/cos))
    # An inferred gentle bend gives the forks depth while preserving registration.
    offset = 8*np.sin((238-y)*.025)+.04*(x-1600)
    depth = thickness[iy,ix]
    offset = np.maximum(offset, (.2-base[:,2])/sin+depth)
    eye = np.array([0,-cos,sin])
    vertices = np.concatenate((base+(offset+depth)[:,None]*eye,base+(offset-depth)[:,None]*eye))
    parts = [[],[],[]];fronts = [[],[],[]];count=len(ix)
    for py,px in zip(*np.where(silhouette)):
        world_y=py+record['box_top_left'][1]
        part=2 if world_y>=210 else (0 if world_y>=112 else 1)
        tl,tr,bl,br=lookup[py,px],lookup[py,px+1],lookup[py+1,px],lookup[py+1,px+1]
        parts[part].extend([(tl,bl,br,tr),(tl+count,tr+count,br+count,bl+count)])
        fronts[part].extend([True,False])
        # Close only the exterior outline, not the internal source-part cuts.
        for dy,dx,p,q in [(-1,0,tl,tr),(1,0,br,bl),(0,-1,bl,tl),(0,1,tr,br)]:
            yy,xx=py+dy,px+dx
            if not (0<=yy<height and 0<=xx<width and silhouette[yy,xx]):
                parts[part].append((p,q,q+count,p+count));fronts[part].append(False)
    np.savez_compressed(ROOT/'native-volume.npz',vertices=vertices,
                        **{f'faces{i}':np.asarray(f,dtype=np.int32) for i,f in enumerate(parts)})
    report=dict(native_mask_index=11,native_mask_sha256=sha(NATIVE/record['png']),
        retained_silhouette_pixels=retained, discarded_disconnected_pixels=int(counts.sum()-retained),
        inferred_corner_bridge_pixels=bridged, maximum_silhouette_repair_pixels=1,
        source_registration='Original 35 degree camera; rounded depth varies only along its rays',
        inference='Rounded cross-sections, concealed backs, branch depth and grounding',
        geometry_npz_sha256=sha(ROOT/'native-volume.npz'))
    (ROOT/'geometry-source.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


def stage():
    import bpy
    sys.path[:0]=[str(HERE),str(EDITOR/'refinement'),str(EDITOR/'refinement/blender')]
    from render_slots import acquire
    from stage_grouping_review import fingerprint
    from source_projection_bake import bake
    from stage_editor_migration import runtime_material
    acquire()
    grouping=EDITOR/'work/needs-review/sherwood-refinement/grouping-review'
    worker=grouping/'grouped-source-only.blend'
    source_record=json.loads((ROOT/'geometry-source.json').read_text())
    assert sha(ROOT/'native-volume.npz')==source_record['geometry_npz_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(worker))
    bpy.context.window.scene=bpy.data.scenes['Sherwood Editor Migration']
    objects=[o for o in bpy.data.collections['Sherwood Working'].objects if o.type=='MESH']
    other=[o for o in objects if o.name not in TARGETS]
    before=fingerprint(other)
    before_membership={o.name:o['asset_group'] for o in objects}
    data=np.load(ROOT/'native-volume.npz')
    for i,name in enumerate(TARGETS):
        obj=bpy.data.objects[name]
        faces=data[f'faces{i}'];indices=np.unique(faces);remap=np.full(len(data['vertices']),-1,int);remap[indices]=np.arange(len(indices))
        mesh=bpy.data.meshes.new(name+' native fork volume')
        mesh.from_pydata(data['vertices'][indices].tolist(),[],remap[faces].tolist());mesh.update()
        obj.data=mesh;obj.matrix_world.identity()
        for face in mesh.polygons:face.use_smooth=True
        obj['refinement_basis']='Native mask 11 branch silhouette; inferred round sections and rear depth'
        obj['geometry_review_status']='pending'
    # Rebuild UVs only for the three explicitly revised meshes. Day excludes the
    # separately animated foliage, whose geometry/materials stay untouched.
    leaves=[o for o in objects if any(m and m.get('foliage_physical_opacity') for m in o.data.materials)]
    visibility={o.name:o.hide_render for o in leaves}
    for obj in leaves:obj.hide_render=True
    source=EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'
    bake('Sherwood',source,ROOT/'bark-reprojection.json',projection_label='exterior',preserve_authored=False,
         receiver_object_names=TARGETS,collection_name='Sherwood Working',
         source_mask_manifest=EDITOR/'work/needs-review/sherwood-refinement/textures/native-mask-audit/compiled-pass-2/source-masks.json',
         provenance_directory=ROOT/'ownership')
    for obj in leaves:obj.hide_render=visibility[obj.name]
    assert fingerprint(other)==before,'Unrelated geometry/UV/material bindings changed'
    assert {o.name:o['asset_group'] for o in objects}==before_membership,'Approved grouping changed'
    bpy.ops.wm.save_as_mainfile(filepath=str(ROOT/'candidate.blend'),compress=True)
    report=dict(status='MODEL_REVIEW_REQUIRED',worker_sha256=sha(ROOT/'candidate.blend'),
        baseline_worker_sha256=sha(worker),changed_objects=TARGETS,grouping_membership_unchanged=True,
        unrelated_geometry_uv_materials_unchanged=True,foliage_unchanged=True,
        source_geometry=source_record,geometry_review='pending',textures_final=False)
    (ROOT/'stage.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--prepare',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:])
    prepare() if args.prepare else stage()
