"""Per-pixel reveal support with an independently visible covered-source fallback."""
import hashlib
from pathlib import Path


def region_record(manifest, directory, patch_id, source, exterior, fallback_nodes, covered_components=None):
    patch = next(p for p in manifest['patches'] if p['id'] == patch_id)
    alpha = (Path(directory) / patch['graphic']['alpha']).resolve(strict=True)
    def sha(path):
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()
    return {'alpha_path': str(alpha), 'alpha_sha256': sha(alpha),
            'bbox': patch['graphic']['bbox'], 'source_sha256': sha(source),
            'state': 'revealed/' + patch_id,
            'fallback': {'source_path': str(exterior), 'source_sha256': sha(exterior),
                         'state': 'covered/' + patch_id, 'projection_label': 'exterior',
                         'occluder_nodes': sorted(fallback_nodes),
                         **({'include_components':covered_components} if covered_components else {})}}


class ProjectionRegion:
    def __init__(self, record, source_hash, source_size, objects, mask_manifest=None, available_objects=None):
        import bpy
        import numpy as np
        from mathutils.bvhtree import BVHTree
        self.record = record
        if not record.get('state') or record['source_sha256'] != source_hash:
            raise ValueError('Reveal region source hash/state mismatch')
        fallback = record['fallback']
        if not fallback.get('state') or not fallback.get('projection_label'):
            raise ValueError('Reveal fallback requires explicit source state and label')
        def load(path, expected):
            if hashlib.sha256(Path(path).read_bytes()).hexdigest() != expected:
                raise ValueError('Regional source evidence changed: ' + str(path))
            image = bpy.data.images.load(str(path), check_existing=False)
            try:
                return np.asarray(image.pixels[:],dtype=np.float32).reshape(image.size[1],image.size[0],4)
            finally:
                bpy.data.images.remove(image)
        self.alpha = load(record['alpha_path'],record['alpha_sha256'])[::-1,:,0]
        left,top,width,height = record['bbox']
        if self.alpha.shape != (height,width):
            raise ValueError('Reveal alpha dimensions differ from bounding box')
        self.pixels = load(fallback['source_path'],fallback['source_sha256'])
        if self.pixels.shape[:2] != (source_size[1],source_size[0]):
            raise ValueError('Regional fallback source dimensions differ')
        self.constraints = None
        if mask_manifest:
            from occlusion_constraints import SourceMaskConstraints
            self.constraints = SourceMaskConstraints(mask_manifest,fallback['projection_label'],
                fallback['source_sha256'],source_size)
        selected=set(fallback['occluder_nodes'])
        fallback_objects=list(objects)
        if fallback.get('include_components'):
            from reveal_components import filter_occluders
            catalog=list(available_objects) if available_objects is not None else list(objects)
            without=filter_occluders(catalog,fallback['include_components'],
                projection_label='interior-'+record['state'].removeprefix('revealed/'),available_objects=catalog)
            fallback_objects.extend(o for o in catalog if o not in without and o not in fallback_objects)
        if selected - {o.get('source_node') for o in fallback_objects}:
            raise ValueError('Absent regional fallback occluders')
        vertices, triangles, self.owners = [], [], []
        depsgraph=bpy.context.evaluated_depsgraph_get()
        for obj in fallback_objects:
            if obj.get('source_node') not in selected:
                continue
            evaluated=obj.evaluated_get(depsgraph)
            mesh=evaluated.to_mesh()
            try:
                mesh.calc_loop_triangles()
                offset=len(vertices)
                vertices.extend(obj.matrix_world @ v.co for v in mesh.vertices)
                triangles.extend(tuple(offset+i for i in t.vertices) for t in mesh.loop_triangles)
                self.owners.extend(obj for _ in mesh.loop_triangles)
            finally:
                evaluated.to_mesh_clear()
        if not triangles:
            raise ValueError('Empty regional fallback occlusion geometry')
        self.tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)

    def contains(self, x, top_y):
        left,top,width,height=self.record['bbox']
        return bool(0 <= x-left < width and 0 <= top_y-top < height and
                    self.alpha[top_y-top,x-left] > 0)

    def membership(self, sx, sy, source_height):
        import numpy as np
        return np.asarray([self.contains(int(x),source_height-1-int(y)) for x,y in zip(sx,sy)])
