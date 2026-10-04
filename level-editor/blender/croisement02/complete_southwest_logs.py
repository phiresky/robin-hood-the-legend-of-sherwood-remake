"""Add omitted source-visible timber while preserving the approved log meshes.

The original baked model is read-only. New closed wood members receive native
source projection; no generated texture or existing material is rewritten.
"""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY,replace_mesh
from render_slots import acquire,release
from refinement_workspace import prepare,validate,_render,_geometry
from source_projection_bake import bake
from audit_candidates import audit
from render_tree import render_workspace

ASSET='croisement02-southwest-log-pile'
# Pixel center lines and perpendicular silhouette radii traced on native artwork.
MEMBERS=[(135, 'Right lower timber fork', [(215, 1081, 7), (233, 1078, 5), (252, 1076, 3), (264, 1070, 3)], 5, None),
 (135,
  'Upper rear log',
  [(23, 1063, 4),
   (35, 1060, 6),
   (45, 1050, 8),
   (55, 1051, 8),
   (65, 1049, 8),
   (80, 1056, 7),
   (116, 1061, 6),
   (164, 1057, 7),
   (208, 1056, 7),
   (250, 1055, 7),
   (280, 1055, 7),
   (290, 1054, 7),
   (295, 1056, 2)],
  15,
  None),
 (134,
  'Lower trailing timber',
  [(77, 1078, 5),
   (105, 1084, 6),
   (139, 1091, 6),
   (174, 1099, 6),
   (201, 1100, 5),
   (225, 1102, 3),
   (230, 1102, 1)],
  0,
  None),
 (134,
  'Low foreground log',
  [(22, 1089, 4), (38, 1088, 7), (69, 1091, 7), (103, 1095, 7), (133, 1098, 6), (172, 1106, 2.8)],
  0,
  None),
 (130,
  'Bent foreground timber continuation',
  [(7, 1089, 2.5),
   (24, 1098, 4),
   (47, 1108, 5.5),
   (77, 1118, 6),
   (97, 1128, 5),
   (113, 1139, 3),
   (128, 1149, 0.7)],
  0,
  None),
 (134, 'Left broken stub', [(52, 1057, 5), (46, 1047, 4), (46, 1040, 4)], 0, (1057, 18)),
 (134, 'Second broken stub', [(72, 1058, 4), (66, 1049, 3), (65, 1042, 3)], 0, (1058, 18))]


def sweep(obj,path,lift,anchor,reference_axis=None):
    vertices=[];faces=[];n=20
    for i,(x,y,r) in enumerate(path):
        previous=path[max(0,i-1)];after=path[min(len(path)-1,i+1)];dx,dy=after[0]-previous[0],after[1]-previous[1];length=math.hypot(dx,dy)
        perpendicular=Vector((-dy/length,-dx/length*SIN,-dx/length*COS))
        z=r+lift if anchor is None else anchor[1]+(anchor[0]-y)/COS
        center=Vector((x,-(y+z*COS)/SIN,z))
        if reference_axis is not None:
            a,b=reference_axis;t=(x-a.x)/(b.x-a.x)
            screen=Vector((x,-y*SIN,-y*COS))
            center=screen+RAY*a.lerp(b,t).dot(RAY)
        for j in range(n):
            angle=math.tau*j/n;vertices.append(tuple(center+r*(math.cos(angle)*perpendicular+math.sin(angle)*RAY)))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(path)-1):
        a=i*n;b=(i+1)*n
        faces.extend((a+j,a+(j+1)%n,b+(j+1)%n,b+j) for j in range(n))
    faces.append(tuple((len(path)-1)*n+j for j in range(n)))
    report=replace_mesh(obj,vertices,faces,materials=list(obj.data.materials))
    for face in obj.data.polygons:face.use_smooth=len(face.vertices)==4
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));report.update(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.to_mesh(obj.data);bm.free()
    if report['nonmanifold_edges'] or report['degenerate_faces']:raise ValueError('Invalid closed wood')
    return dict(**report,path=path,depth='Round unseen cross section inferred from observed width',source_anchor=anchor)


def main(directory):
    old=OUT/'scenery-round-1/assets'/ASSET
    baked=OUT/'texture-fill-round-1'/ASSET/'experiment/bake-v1/worker.blend'
    protected={str(p):sha(p) for p in [old/'model.blend',baked]}
    workspace=directory/'assets'/ASSET
    directory.mkdir(parents=True,exist_ok=False)
    traces=json.loads((OUT/'southwest-log-revision/native-traces.json').read_text())
    if traces['source_sha256']!=sha(OUT/'animation-references/composite-frame-0.png'):raise ValueError('Stale source trace')
    for i,digest in traces['mask_sha256'].items():
        if sha(OUT/f'baseline/masks/{int(i):06}.png')!=digest:raise ValueError('Stale native wood mask')
    members=list(MEMBERS)
    members=[(source,label,traces['foreground_log_profile'] if source==130 else path,lift,anchor) for source,label,path,lift,anchor in members]
    members += [(135,f'Native upper twig{i:02}',path,0,(1054,23)) for i,path in enumerate(traces['branches'])]
    write_json(directory/'source-traces.json',traces)
    bpy.ops.wm.open_mainfile(filepath=str(baked));bpy.context.preferences.filepaths.save_version=0
    # Freeze a conventional input packet, then reopen the immutable approved
    # appearance before adding members. Preparation's source-only rendering must
    # never become an accidental replacement of the accepted filled materials.
    prepare(workspace,asset_id=ASSET,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',
        source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=1.5,lighting=json.loads((old/'workspace.json').read_text())['lighting'])
    bpy.ops.wm.open_mainfile(filepath=str(baked));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];before={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH'}
    original={int(o['source_node'].split('-')[-1]):o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==ASSET}
    new=[];records=[]
    for source,label,path,lift,anchor in members:
        obj=bpy.data.objects.new(label,bpy.data.meshes.new(label));collection.objects.link(obj)
        for key,value in dict(source_node=f'building-{source:03}',source_obstacle=source,asset_group=ASSET,asset_name='Southwest Log Pile',part_name=label).items():obj[key]=value
        for material in original[source].data.materials:obj.data.materials.append(material)
        axis=None
        if source==130:
            points=[original[source].matrix_world@v.co for v in original[source].data.vertices];half=len(points)//2
            axis=(sum(points[:half],Vector())/half,sum(points[half:],Vector())/half)
        records.append(dict(object=obj.name,source_node=obj['source_node'],**sweep(obj,path,lift,anchor,axis)));new.append(obj)
    config=json.loads((workspace/'workspace.json').read_text())
    report=bake('Croisement02',config['source_path'],workspace/'projection/added-wood.json',receiver_nodes=sorted({o['source_node'] for o in new}),receiver_object_names=[o.name for o in new],occluder_nodes=sorted({o['source_node'] for o in new}),projection_label='exterior',elevation_deg=35,preserve_authored=False,source_mask_manifest=config['source_mask_manifest'])
    after={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH' and o.name in before}
    if before!=after:raise ValueError('Existing mesh/UV/material/texture changed during additive fill')
    _render(config,workspace/'modified',workspace/'input/views.json')
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace/'model.blend'));validate(workspace)
    inspection=workspace/'inspection';inspection.mkdir(exist_ok=True)
    source_domain=inspection/'source-domain';source_domain.mkdir();level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());union=np.zeros((1152,1792),bool)
    for index in [102,103]:
        r=level['masks'][index];x,y=r['box_top_left'];w,h=r['box_size'];union[y:y+h,x:x+w]|=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'))>0
    rgba=Image.open(config['source_path']).convert('RGBA');rgba.putalpha(Image.fromarray(union.astype('uint8')*255));rgba.save(source_domain/'complete-source.png');write_json(source_domain/'partition.json',dict(native_bbox=[0,0,1792,1152],native_masks=[102,103],note='Union of both native log silhouettes; no invented actor obstacle or ownership expansion.'))
    write_json(inspection/'refinement.json',dict(asset_id=ASSET,model_sha256=sha(workspace/'model.blend'),mask=102,source_packet=str(source_domain/'partition.json'),new_members=records,status='isolated additive geometry candidate; self-review pending',limitations=['All approved original geometry, UVs, material nodes and image pixels remain unchanged. New closed timber can overlap existing log interiors at contacts.','The lower foreground log and branch/stub silhouettes are source observed; exact hidden depth and attachment depth are inferred.','Only added timber receives native source projection. New unseen surfaces are neutral and need geometry approval before fill.','No canonical workspace, user approval or published model is replaced.']))
    write_json(inspection/'preservation.json',dict(status='PASS',protected_files=protected,existing_mesh_appearance_identical=True,existing_objects=len(before),added_objects=len(new),original_object_fingerprints=before,bake_report_sha256=sha(workspace/'projection/added-wood.json')))
    audit(workspace);render_workspace(workspace,384,release_slot=False)
    if any(sha(Path(p))!=expected for p,expected in protected.items()):raise ValueError('Immutable approved files changed')
    print(workspace,flush=True)

def verify_saved(workspace):
    evidence=json.loads((workspace/'inspection/preservation.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'))
    expected=evidence['original_object_fingerprints']
    actual={o.name:_geometry(o,protect_appearance=True) for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.name in expected}
    if actual!=expected:raise ValueError('Reopened approved geometry or appearance changed')
    if any(sha(Path(p))!=digest for p,digest in evidence['protected_files'].items()):raise ValueError('Approved baseline file changed')
    write_json(workspace/'inspection/reopened-preservation.json',dict(status='PASS',model_sha256=sha(workspace/'model.blend'),preservation_sha256=sha(workspace/'inspection/preservation.json'),existing_objects_unchanged=len(actual),scope='Existing vertex positions, faces, UVs, transforms, materials, shader nodes and packed images are unchanged.'))
    print('Reopened preservation PASS',workspace,flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,default=OUT/'southwest-log-revision/v9');parser.add_argument('--verify-only',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire()
    try:
        if args.verify_only:verify_saved(args.verify_only.resolve())
        else:
            main(args.output.resolve())
            verify_saved(args.output.resolve()/'assets'/ASSET)
    finally:release()
