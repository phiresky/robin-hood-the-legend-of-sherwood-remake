"""Create the non-destructive Sherwood inspection scene through Blender MCP."""

import math
from pathlib import Path

import bpy
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/sherwood-refinement'
if 'Sherwood Refinement' in bpy.data.scenes:
    raise RuntimeError('Sherwood scene already exists; open its saved checkpoint instead')
scene=bpy.data.scenes.new('Sherwood Refinement')
bpy.context.window.scene=scene
bpy.ops.import_scene.gltf(filepath=str(ROOT/'level-editor/library/scenes/sherwood-volumes.scene.glb'))
baseline=bpy.data.collections.new('00 Baseline - original obstacle reconstruction')
scene.collection.children.link(baseline)
for obj in list(scene.objects):
    baseline.objects.link(obj)
    for collection in list(obj.users_collection):
        if collection!=baseline:collection.objects.unlink(obj)
baseline.hide_render=True
baseline.hide_viewport=True
working=bpy.data.collections.new('01 Refinement - working copy')
scene.collection.children.link(working)
copies={}
for obj in baseline.objects:
    copy=obj.copy()
    if obj.data:copy.data=obj.data.copy()
    working.objects.link(copy)
    copies[obj]=copy
    copy['source_obstacle']=obj.name
for obj,copy in copies.items():
    if obj.parent:copy.parent=copies[obj.parent]
views=bpy.data.collections.new('02 Inspection cameras')
scene.collection.children.link(views)
sin=math.sin(math.radians(35))


def camera(name,px,py,span,yaw=0,elevation=35):
    data=bpy.data.cameras.new(name)
    obj=bpy.data.objects.new(name,data)
    views.objects.link(obj)
    target=Vector((px,-py/sin,0))
    yaw,elevation=math.radians(yaw),math.radians(elevation)
    obj.location=target+Vector((math.sin(yaw)*math.cos(elevation),-math.cos(yaw)*math.cos(elevation),math.sin(elevation)))*3000
    obj.rotation_euler=(target-obj.location).to_track_quat('-Z','Y').to_euler()
    data.type='ORTHO'
    data.ortho_scale=span
    data.clip_end=12000
    return obj


scene.camera=camera('Reference Camera',960,544,1920)
reference=bpy.data.images.load(str(ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'),check_existing=True)
reference.pack()
background=scene.camera.data.background_images.new()
background.image=reference
background.alpha=0.5
background.display_depth='FRONT'
scene.camera.data.show_background_images=False
camera('Orbit - east 40 degrees',960,544,2100,40,45)
camera('Orbit - west 40 degrees',960,544,2100,-40,45)
camera('Plan - terrain and footprints',960,544,2300,0,90)
camera('Detail - treehouse and bridges',530,225,1120)
camera('Detail - central oak and hut',980,465,650)
camera('Detail - camp furniture',520,640,560)
camera('Detail - riverbank',1500,690,780)
orbit=camera('Detail - ladder oak orbit',490,425,950)
target=Vector((490,-740,245))
orbit.location=target+Vector((0.48,-0.76,0.44))*3000
orbit.rotation_euler=(target-orbit.location).to_track_quat('-Z','Y').to_euler()
scene.render.engine='BLENDER_EEVEE'
scene.render.resolution_x=1920
scene.render.resolution_y=1088
scene.render.resolution_percentage=100
scene.render.image_settings.file_format='PNG'
scene.view_settings.view_transform='Standard'
scene.view_settings.look='None'
scene.view_settings.exposure=0
scene.view_settings.gamma=1
scene.display.shading.light='STUDIO'
scene.display.shading.color_type='SINGLE'
scene.display.shading.single_color=(0.65,0.65,0.65)
scene.display.shading.show_cavity=True
scene.display.shading.cavity_type='BOTH'
scene.display.shading.show_shadows=True
for i,obj in enumerate(views.objects,1):
    marker=scene.timeline_markers.new(obj.name,frame=i)
    marker.camera=obj
scene.frame_start=1
scene.frame_end=9
scene.frame_set(1)
for area in bpy.context.screen.areas:
    if area.type=='VIEW_3D':
        area.spaces.active.clip_end=12000
        area.spaces.active.region_3d.view_perspective='CAMERA'
OUT.mkdir(parents=True,exist_ok=True)
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'sherwood-refinement.blend'))
result={'file':bpy.data.filepath,'cameras':len(views.objects)}
