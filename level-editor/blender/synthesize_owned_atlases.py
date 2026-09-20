"""Resynthesize existing ownership atlases without repeating visibility rays."""
import json
from pathlib import Path
import numpy as np
from source_texture_fill import donor_patch, fill_island, choose_donor, synthesize_tiles, prune_donors


def synthesize(map_name, output_dir):
    import bpy
    output = Path(output_dir)
    output.mkdir(parents=True, exist_ok=True)
    pending, donors = [], {}
    for obj in bpy.data.collections[map_name+' Working'].all_objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        mesh = obj.data
        mesh.calc_loop_triangles()
        by_face = {}
        for triangle in mesh.loop_triangles:
            by_face.setdefault(triangle.polygon_index, []).append(triangle)
        normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
        for slot, mat in enumerate(mesh.materials):
            if not mat or mat.get('source_ownership_fill') != 'synthesized':
                continue
            label = mat['source_ownership_label']
            texture = next(n for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE')
            image = texture.image
            width, height = image.size
            atlas = np.empty(width*height*4, dtype=np.float32)
            image.pixels.foreach_get(atlas)
            atlas = atlas.reshape(height, width, 4)
            uv = mesh.uv_layers['Owned source / '+label]
            group = (label, obj.get('asset_group') or obj.name)
            islands = []
            for face in mesh.polygons:
                if face.material_index != slot:
                    continue
                coords = np.array([(uv.data[lid].uv.x*width, uv.data[lid].uv.y*height) for lid in face.loop_indices])
                left, bottom = np.rint(coords.min(axis=0)).astype(int)
                right, top = np.rint(coords.max(axis=0)).astype(int)
                if right <= left or top <= bottom:
                    continue
                yy, xx = np.mgrid[bottom:top, left:right]
                inside = np.zeros(xx.shape, dtype=bool)
                for triangle in by_face[face.index]:
                    a,b,c = [(uv.data[lid].uv.x*width, uv.data[lid].uv.y*height) for lid in triangle.loops]
                    det = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
                    if abs(det) < 1e-10:
                        continue
                    wa = ((b[1]-c[1])*(xx+.5-c[0])+(c[0]-b[0])*(yy+.5-c[1]))/det
                    wb = ((c[1]-a[1])*(xx+.5-c[0])+(a[0]-c[0])*(yy+.5-c[1]))/det
                    inside |= (wa >= 0) & (wb >= 0) & (wa+wb <= 1)
                tile = atlas[bottom:top, left:right]
                patch = donor_patch(tile, inside & (tile[:,:,3] > .5))
                normal_z = abs((normal_matrix @ face.normal).normalized().z)
                if patch is not None:
                    donors.setdefault(group, []).append((patch,normal_z,obj.name))
                islands.append((max(0,left-2),max(0,bottom-2),min(width,right+2),min(height,top+2),normal_z))
            pending.append((obj,image,group,islands))
    donors = {key:prune_donors(value) for key,value in donors.items()}
    selected = [choose_donor(donors.get(group,[]),island[4],obj.name)
                for obj,image,group,islands in pending for island in islands]
    tiles, synthesis = synthesize_tiles(selected,output/'cache')
    report = {'synthesis':synthesis,'objects':[],'geometry_changed':False}
    for obj,image,group,islands in pending:
        width,height = image.size
        atlas = np.empty(width*height*4,dtype=np.float32)
        image.pixels.foreach_get(atlas)
        atlas = atlas.reshape(height,width,4)
        observed = atlas[:,:,3] > .5
        before = atlas[observed].copy()
        count = 0
        for left,bottom,right,top,normal_z in islands:
            count += fill_island(atlas[bottom:top,left:right],donors.get(group,[]),normal_z,obj.name,(left,bottom),tiles)
        if not np.array_equal(atlas[observed],before):
            raise RuntimeError('Synthesis modified observed source pixels')
        image.pixels.foreach_set(atlas.ravel())
        image.update()
        image.pack()
        report['objects'].append({'object':obj.name,'projection_label':group[0],
                                  'inferred_texels_including_padding':count,
                                  'missing_donor':not donors.get(group)})
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    return report
