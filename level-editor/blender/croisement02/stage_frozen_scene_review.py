"""Assemble a frozen private scene with strict textures and separate ground receiver."""
import argparse
import json
import sys
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
import stage_review_scene as stage
from evidence_io import sha,write_json
from render_slots import acquire,release
from approved_texture_stage import geometry,appearance


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('snapshot',type=Path);parser.add_argument('output',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    snapshot=args.snapshot.resolve();output=args.output.resolve();manifest=json.loads((snapshot/'snapshot.json').read_text())
    if sha(snapshot/'catalog.json')!=manifest['catalog_sha256']:raise ValueError('Frozen catalog changed')
    for row in manifest['workers']:
        if 'frozen' in row and sha(Path(row['frozen'])/'model.blend')!=row['model_sha256']:raise ValueError('Frozen worker changed')
    stage.reviewed_catalog=lambda:snapshot/'catalog.json'
    stage.tree_workspace=lambda key:Path(manifest['trees'][str(key)])
    stage.scenery_workspace=lambda key:Path(manifest['scenery'][key])
    renderer=stage.render_review;stage.render_review=lambda *args,**kwargs:None
    stage.main(output,snapshot/'texture-decisions.json')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(snapshot/'ground.blend'))
        source,=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-ground-receiver']
        ground_name=source.name;ground_geometry=geometry(source);ground_appearance=appearance(source)
        bpy.ops.wm.open_mainfile(filepath=str(output/'scene.blend'))
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        collection=bpy.data.collections['Croisement02 Working'];old,=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='ground']
        with bpy.data.libraries.load(str(snapshot/'ground.blend'),link=False) as (src,target):target.objects=[ground_name]
        ground=target.objects[0];collection.objects.link(ground)
        ancestor=ground.parent
        while ancestor is not None:
            if ancestor.type=='MESH':raise ValueError('Ground parent is another mesh')
            if ancestor.name not in scene.objects:collection.objects.link(ancestor)
            ancestor=ancestor.parent
        bpy.data.objects.remove(old,do_unlink=True);bpy.context.view_layer.update()
        if geometry(ground)!=ground_geometry or appearance(ground)!=ground_appearance:raise ValueError('Ground receiver changed on import')
        report=json.loads((output/'assembly.json').read_text());report['pre_ground_scene_sha256']=report['model_sha256']
        bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(output/'scene.blend'),compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(output/'scene.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        for record in report['assets']:
            if not record.get('texture_approval'):continue
            for evidence in record['objects']:
                obj=bpy.data.objects[evidence['imported_name']]
                if appearance(obj)!=evidence['texture_appearance_sha256'] or geometry(obj)!=evidence['texture_geometry_sha256']:raise ValueError('Ground integration changed approved texture receiver')
        report.update(model_sha256=sha(output/'scene.blend'),snapshot_sha256=sha(snapshot/'snapshot.json'),ground=dict(worker=manifest['ground_worker'],model_sha256=manifest['ground_model_sha256'],geometry_signature=ground_geometry,appearance_signature=ground_appearance,approval='pending'),texture_omissions=manifest['texture_omissions'],remaining=['Pending geometry is included for private spatial review, not user approval.','Hidden ground is neutral; state targets and transitions remain separate pending assemblies.','Final first-hit and contact audit is separate from these complete-scene views.'])
        write_json(output/'assembly.json',report)
        renderer(scene,output,report['model_sha256'])
    finally:release()


if __name__=='__main__':main()
