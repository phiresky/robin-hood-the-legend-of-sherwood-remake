"""Validate the finished branch/leaf geometry and save review views through MCP."""
import json
from pathlib import Path
import bpy
import bmesh

s=bpy.data.scenes['Sherwood Refinement'];bpy.context.window.scene=s
c=bpy.data.collections['10 Animated foliage - curved canopy shells']
out=Path(bpy.data.filepath).parent/'branch-canopies';out.mkdir(exist_ok=True)
report={'objects':[],'errors':[]}
checked=list(c.objects)+[o for o in bpy.data.collections['08 Refined forest - trunks roots and branches'].objects if 'crown_fork_height_world' in o]
for obj in checked:
    bm=bmesh.new();bm.from_mesh(obj.data)
    bad=sum(not e.is_manifold for e in bm.edges);zero=sum(f.calc_area()<1e-9 for f in bm.faces)
    row={'object':obj.name,'vertices':len(bm.verts),'faces':len(bm.faces),'nonmanifold_edges':bad,'zero_area_faces':zero}
    if bad or zero:report['errors'].append(row)
    report['objects'].append(row);bm.free()
materials={m for o in c.objects for m in o.data.materials}
report['drivers']=[{'material':m.name,'valid':f.driver.is_valid,'expression':f.driver.expression}
                   for m in materials if m.node_tree.animation_data for f in m.node_tree.animation_data.drivers]
report['leaf_sprays']=sum(o.get('sprays',0) for o in c.objects)
report['leaves']=sum(o.get('individual_leaves',0) for o in c.objects)
report['crown_sectors']=sum('individual leaf sprays' in o.name for o in c.objects)
report['supporting_trees']=len({o['supporting_tree'] for o in c.objects})
visibility={o:o.hide_render for o in c.objects}
ground=bpy.data.objects['Terrain - lightly undulating clearing']
old_ground=ground.hide_render;old_override=s.view_layers[0].material_override
wire=next(m for m in bpy.data.materials if m.name.startswith('INSPECTION - dark topology lines'))
try:
    for name,frame,mode in [
        ('01-original-camera',1,'texture'),('02-solid-east',2,'solid'),('03-solid-west',3,'solid'),
        ('04-textured-east',2,'texture'),('05-textured-west',3,'texture'),('06-solid-overhead',4,'solid'),
        ('07-foreground-solid',10,'solid'),('08-foreground-textured',10,'texture'),
        ('09-foreground-branches',10,'branches'),('10-forest-branches',2,'branches'),
        ('11-foreground-wireframe',10,'wire')]:
        s.frame_set(frame)
        s.render.resolution_x=1200 if frame==10 else 1920
        s.render.resolution_y=1200 if frame==10 else 1088
        s.render.engine='BLENDER_EEVEE' if mode in ['texture','wire'] else 'BLENDER_WORKBENCH'
        s.view_layers[0].material_override=wire if mode=='wire' else None
        ground.hide_render=mode=='wire'
        for obj in c.objects:obj.hide_render=mode=='branches' and 'limbs forks' not in obj.name
        s.render.filepath=str(out/(name+'.png'));bpy.ops.render.render(write_still=True)
finally:
    for obj,value in visibility.items():obj.hide_render=value
    ground.hide_render=old_ground;s.view_layers[0].material_override=old_override
    s.render.resolution_x=1920;s.render.resolution_y=1088
    s.render.engine='BLENDER_WORKBENCH';s.frame_set(10)
a=bpy.data.scenes['Sherwood Animation Reference'];bpy.context.window.scene=a
for frame in [1,33]:
    a.frame_set(frame);a.render.filepath=str(out/f'animation-{frame:02}.png');bpy.ops.render.render(write_still=True)
a.frame_set(1);bpy.context.window.scene=s
bpy.ops.file.pack_all();bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
(out/'geometry-validation.json').write_text(json.dumps(report,indent=2))
result={k:v for k,v in report.items() if k!='objects'}
