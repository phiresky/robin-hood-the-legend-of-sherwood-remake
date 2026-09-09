"""Render each crown's actual alpha independently for reference silhouette fitting."""
from pathlib import Path
import bpy

s=bpy.data.scenes['Sherwood Refinement'];bpy.context.window.scene=s
c=bpy.data.collections['10 Animated foliage - curved canopy shells']
out=Path(bpy.data.filepath).parent/'branch-canopies';out.mkdir(exist_ok=True)
collections={col:col.hide_render for col in s.collection.children}
objects={o:o.hide_render for o in c.objects}
state=(s.frame_current,s.render.engine,s.render.film_transparent)
files=[]
try:
    s.frame_set(1);s.render.engine='BLENDER_EEVEE';s.render.film_transparent=True
    for col in s.collection.children:col.hide_render=col.name not in ['02 Inspection cameras',c.name]
    for profile in sorted({o['profile'] for o in c.objects}):
        for o in c.objects:o.hide_render=o['profile']!=profile or 'limbs forks' in o.name
        s.render.filepath=str(out/(profile.split(' - ')[1].lower()+'-alpha.png'))
        bpy.ops.render.render(write_still=True);files.append(s.render.filepath)
finally:
    for col,value in collections.items():col.hide_render=value
    for obj,value in objects.items():obj.hide_render=value
    s.frame_set(state[0]);s.render.engine=state[1];s.render.film_transparent=state[2]
result={'renders':files}
