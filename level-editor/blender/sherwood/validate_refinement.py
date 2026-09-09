"""Inspect generated geometry and render comparable baseline/refined views."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
sys.path.insert(0,str(Path(__file__).parent))
from paths import OUT

OUT.mkdir(parents=True,exist_ok=True)
scene=bpy.data.scenes['Sherwood Refinement']
bpy.context.window.scene=scene
report={'collections':{},'errors':[]}
for c in scene.collection.children:
    if not c.name[:2].isdigit() or int(c.name[:2])<3 or c.name[:2] in ['07','11']:continue
    counts={'objects':0,'vertices':0,'faces':0,'open_surfaces':0}
    for obj in c.objects:
        if obj.type!='MESH':continue
        bm=bmesh.new();bm.from_mesh(obj.data)
        counts['objects']+=1;counts['vertices']+=len(bm.verts);counts['faces']+=len(bm.faces)
        zero=sum(f.calc_area()<1e-8 for f in bm.faces)
        if obj.get('open_surface'):
            counts['open_surfaces']+=1
            bad=sum(not e.is_manifold and not e.is_boundary for e in bm.edges)
        else:
            bad=sum(not e.is_manifold for e in bm.edges)
            if not bad and bm.calc_volume(signed=True)<0:
                bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data)
        if bad or zero:report['errors'].append({'object':obj.name,'nonmanifold':bad,'zeroarea':zero})
        bm.free()
    report['collections'][c.name]=counts
report['drivers']=[{'material':m.name,'expression':f.driver.expression,'valid':f.driver.is_valid}
                   for m in bpy.data.materials if m.node_tree and m.node_tree.animation_data
                   for f in m.node_tree.animation_data.drivers]
report['unpacked_images']=[i.name for i in bpy.data.images if i.source=='FILE' and not i.packed_file and i.users]

def render(name,frame,engine):
    scene.frame_set(frame);scene.render.engine=engine
    scene.render.filepath=str(OUT/name)
    bpy.ops.render.render(write_still=True)

visibility={c:c.hide_render for c in scene.collection.children}
try:
    for c in scene.collection.children:
        c.hide_render=not c.name.startswith(('00 ','02 '))
    render('baseline-bare-map.png',1,'BLENDER_EEVEE')
    for c,state in visibility.items():c.hide_render=state
    bpy.data.collections['10 Animated foliage - curved canopy shells'].hide_render=True
    bpy.data.collections['11 Authored ambient overlay references'].hide_render=True
    render('refined-bare-map.png',1,'BLENDER_EEVEE')
    render('refined-structure-orbit.png',2,'BLENDER_WORKBENCH')
    render('refined-river-close.png',8,'BLENDER_EEVEE')
    for c,state in visibility.items():c.hide_render=state
    render('refined-composite.png',1,'BLENDER_EEVEE')
    render('refined-foliage-orbit.png',3,'BLENDER_EEVEE')
finally:
    for c,state in visibility.items():c.hide_render=state
    scene.frame_set(1);scene.render.engine='BLENDER_EEVEE'

animation=bpy.data.scenes['Sherwood Animation Reference']
bpy.context.window.scene=animation
for frame in [1,33]:
    animation.frame_set(frame);animation.render.filepath=str(OUT/f'animation-{frame:02}.png')
    bpy.ops.render.render(write_still=True)
animation.frame_set(1)
bpy.context.window.scene=scene
bpy.ops.file.pack_all()
(OUT/'geometry-validation.json').write_text(json.dumps(report,indent=2))
bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
result=report
