"""Isolated north-tower source authority and fixed-camera review packet."""
from pathlib import Path
import sys,json,hashlib
ROOT=Path(__file__).resolve().parents[2]
W=ROOT/'level-editor/work/derby-refinement/round-2/assets/derby-great-keep'
OUT=W/'component-followups/north'
NODES={f'building-{n:03}' for n in [163,164,165,166,171,172,173,174,175]}
def masks():
    from PIL import Image,ImageDraw
    OUT.mkdir(parents=True,exist_ok=True)
    inventory=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/manifest.json'
    records=json.loads(inventory.read_text())['masks']
    source=Image.open(W/'reference/mission-patches/H03_Der_MK-initial.png').convert('RGB')
    ids=[129,131,140,141,142,143,144,168]
    sheet=Image.new('RGB',(1408,1300))
    for i,index in enumerate(ids):
        r=records[index]; mask=Image.new('L',source.size)
        mask.paste(Image.open(inventory.parent/r['png']),r['box_top_left'])
        tint=Image.composite(Image.blend(source,Image.new('RGB',source.size,'magenta'),.55),source,mask)
        crop=tint.crop((834,27,1186,677)); ImageDraw.Draw(crop).text((2,2),f"Mask {index} layer {r['layer']}/{r['layer_index']}",fill='white')
        sheet.paste(crop,((i%4)*352,(i//4)*650))
    sheet.save(OUT/'native-associations.png')
def review():
    sys.path[:0]=['/usr/lib/python3.14','/usr/lib/python3.14/lib-dynload','/usr/lib/python3.14/site-packages',str(ROOT/'level-editor/blender')]
    import bpy
    from refinement_workspace import _geometry
    from refinement_review import render_review
    from source_projection_bake import bake
    from occlusion_constraints import evidence_record
    OUT.mkdir(parents=True,exist_ok=True)
    scene=bpy.data.scenes['Derby Refinement']; bpy.context.window.scene=scene
    objects=list(bpy.data.collections['Derby Working'].objects)
    before={o.name:_geometry(o) for o in bpy.data.objects}
    owned=[o for o in objects if o.type=='MESH' and o.get('source_node') in NODES]
    (OUT/'geometry-inspection.json').write_text(json.dumps([{'name':o.name,'node':o.get('source_node'),'vertices':len(o.data.vertices),'faces':len(o.data.polygons),'properties':{k:v for k,v in o.items() if isinstance(v,(str,int,float,bool))}} for o in owned],indent=2))
    frame=json.loads((W/'inspection/components/derby-keep-north-tower/views.json').read_text())
    for layer in frame['projection_layers']:
        if layer['source_path']==frame['source_image']:layer['projection_label']='exterior'
        elif 'building-223' in layer['receiver_nodes']:layer['projection_label']='interior-patch-000'
        elif 'building-239' in layer['receiver_nodes']:layer['projection_label']='interior-patch-001'
        elif 'building-241' in layer['receiver_nodes']:layer['projection_label']='interior-patch-002'
        elif 'building-249' in layer['receiver_nodes']:layer['projection_label']='interior-patch-003'
        else:layer['projection_label']='exterior' if layer['source_path']==frame['source_image'] else 'interior-other'
    source=Path(frame['source_image'])
    assignments=[{'reviewed':True,'source_node':n,'mask_indices':[131,140,143], 'evidence_scope':'Reviewed union of north spire, west upper facade and full north tower; full scene visibility still required. Not a semantic per-face segmentation.'} for n in sorted(NODES)]
    manifest={'version':1,'mask_inventory':str(ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/manifest.json'),'projections':{'exterior':{'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'state':'H03_Der_MK initial covered. Native131 layer0/93 spire,140 layer10/1 west facade,143 layer0/94 north tower. No patch000/001 interior receivers assigned.','assignments':assignments}}}
    path=OUT/'source-masks.json';path.write_text(json.dumps(manifest,indent=2))
    result=bake('Derby',str(source),OUT/'bake.json',receiver_nodes=sorted(NODES),projection_label='exterior',preserve_authored=False,source_mask_manifest=path)
    assert json.loads((OUT/'bake.json').read_text())['source_mask_state'] is not None
    groups={o:o.get('asset_group') for o in owned}
    try:
        for o in owned:o['asset_group']='derby-keep-north-tower'
        packet=OUT/(sys.argv[sys.argv.index('--packet')+1] if '--packet' in sys.argv else 'approval-final')
        if '--bake-only' not in sys.argv:
            render_review(packet,scene_name='Derby Refinement',collection_name='Derby Working',asset_id='derby-keep-north-tower',source_path=str(source),frame_manifest=frame,projection_layers=frame['projection_layers'],source_mask_manifest=path)
            assert all(r['constrained'] for r in json.loads((packet/'views.json').read_text())['source_constraint_status'])
    finally:
        for o,v in groups.items():o['asset_group']=v
    after={o.name:_geometry(o) for o in bpy.data.objects}
    assert before==after,'Geometry or ownership modified during authority pass'
    proof={'geometry_unchanged':True,'objects_checked':len(before),'owned_nodes':sorted(NODES),'owned_meshes':len(owned),'source_mask_applied_to_bake_and_preview':True,'evidence':evidence_record(path),'limitations':['Union membership and full scene ray visibility constrain source ownership; union is not semantic per-face segmentation.','Attached tower assembly retains concealed attachment surfaces and lower geometry occluded by adjoining hall.']}
    (OUT/'validation.json').write_text(json.dumps(proof,indent=2))
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'))
if __name__=='__main__':
    if '--review' in sys.argv: review()
    else: masks()
