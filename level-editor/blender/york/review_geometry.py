"""Render full-scene grouping checks and export exact mesh triangles for the gallery."""
import hashlib
import argparse
import json
import sys
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'


def main():
    import bpy
    sys.path.insert(0,str(ROOT/'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
    parser=argparse.ArgumentParser()
    parser.add_argument('--grounded',action='store_true')
    parser.add_argument('--geometry-only',action='store_true')
    parser.add_argument('--output',type=Path,default=OUT/'review')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    source=OUT/('grounding/york-grounded.blend' if args.grounded else 'grouped/york-grouped.blend')
    bpy.ops.wm.open_mainfile(filepath=str(source))
    scene=bpy.data.scenes['york Refinement']
    bpy.context.window.scene=scene
    records={}
    for obj in bpy.data.collections['york Working'].objects:
        if obj.type!='MESH' or obj.hide_render:
            continue
        group=obj.get('asset_group','york-terrain')
        record=records.setdefault(group,{'name':obj.get('asset_name','York river and background terrain'),'parts':[]})
        obj.data.calc_loop_triangles()
        record['parts'].append({'source_node':obj['source_node'],
             'component':obj.get('projection_component'),
             'positions':[list(obj.matrix_world@v.co) for v in obj.data.vertices],
             'triangles':[list(t.vertices) for t in obj.data.loop_triangles]})
        color=hashlib.sha256(group.encode()).digest()
        obj.color=tuple(.18+.65*c/255 for c in color[:3])+(1,)
    output=args.output
    output.mkdir(exist_ok=True)
    (output/'scene.json').write_text(json.dumps({'source':str(source),'sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
        'grounded':args.grounded},indent=2)+'\n')
    (output/'geometry.json').write_text(json.dumps(records,separators=(',',':'))+'\n')
    if args.geometry_only:
        print(json.dumps({'groups':len(records),'geometry':str(output/'geometry.json')}))
        return
    scene.render.engine='BLENDER_WORKBENCH'
    scene.display.shading.color_type='OBJECT'
    scene.display.shading.light='STUDIO'
    scene.display.shading.show_shadows=True
    scene.display.shading.show_cavity=True
    scene.display.shading.cavity_type='BOTH'
    scene.render.resolution_percentage=100
    scene.render.resolution_x=1600
    scene.render.resolution_y=1183
    scene.render.image_settings.file_format='PNG'
    for label in ['reference','east','west','plan']:
        scene.camera=bpy.data.objects['york '+label]
        scene.render.filepath=str(output/(label+'.png'))
        bpy.ops.render.render(write_still=True)
    print(json.dumps({'groups':len(records),'review':str(output)}))


if __name__=='__main__':
    main()
