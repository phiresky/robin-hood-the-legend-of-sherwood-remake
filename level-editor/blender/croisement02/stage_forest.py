"""Stage the complete forest grouping and canopy hypotheses without publication."""
import json
import sys
from pathlib import Path
import bpy
from mathutils import Matrix
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,TREES
from forest_layout import CROWNS
from tree_geometry import foliage_packet,crown_geometry
from render_slots import acquire
from refinement_inventory import inventory
from evidence_io import sha


def main():
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement02-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
    for obj in list(collection.all_objects):
        if obj.get('projection_component')=='crown':bpy.data.objects.remove(obj,do_unlink=True)
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());reports=[]
    for animation,centres in CROWNS.items():
        for selected,(mask,cx,cy) in enumerate(centres):
            primary=TREES[mask][0];node=f'building-{primary:03}'
            owner=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')==node)
            points=level['sight_obstacles'][primary]['points'];ground=sum(p['y'] for p in points)/len(points)
            seeds=[(cx+dx,cy+dy) for dx,dy in [(0,-55),(-45,-25),(45,-25),(-65,15),(0,15),(65,15),(-25,55),(25,65)]]
            directory=OUT/f'forest-v4-sources/tree-{mask:02}'
            packet=foliage_packet(animation,seeds,directory,sector=(selected,centres))
            crown=bpy.data.objects.new(owner['asset_name']+' / Crown',bpy.data.meshes.new('Canopy'))
            collection.objects.link(crown);crown.parent=owner.parent;crown.matrix_world=Matrix.Identity(4)
            for key in ('source_node','source_obstacle','asset_group','asset_name','part_name'):crown[key]=owner[key]
            bpy.context.view_layer.update()
            report=crown_geometry(crown,packet,ground,False,depth_ratio=.55)
            reports.append(dict(mask=mask,primary=primary,ground_y=ground,packet=str(directory/'partition.json'),animation=animation,
                                source_bbox=packet['bbox'],asset_id=owner['asset_group'],reference_proxy=report))
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'forest-v4-input.blend'))
    (OUT/'forest-v4-sources/manifest.json').write_text(json.dumps(reports,indent=2)+'\n')
    inventory(OUT/'forest-v4-inventory',collection_name=collection.name,map_name='Croisement02',source_path=OUT/'animation-references/composite-frame-0.png',patch_manifest=OUT/'source-states/layers.json')
    print('Staged',len(reports),'separate tree crown candidates')

if __name__=='__main__':main()
