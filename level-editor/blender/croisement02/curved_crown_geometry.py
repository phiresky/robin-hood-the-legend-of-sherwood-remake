"""Small observed foliage fragments on an irregular full-depth crown envelope."""
import math
from pathlib import Path
from PIL import Image
import numpy as np
from mathutils import Vector
from rounded_interior_geometry import build as volume
from tree_geometry import SIN, COS, RAY, replace_mesh, material, one_sided


def build(obj, packet, ground_y, interior_clusters=600, branch_clumps=False, fragment_depth_jitter=0.):
    if not 0 <= fragment_depth_jitter <= 16:
        raise ValueError("Fragment depth jitter must be in 0..16 source pixels")
    report = volume(obj, packet, ground_y, interior_clusters=interior_clusters, branch_clumps=branch_clumps)
    mesh = obj.data
    fx, fy, fw, fh = packet['bbox']
    cx, cy = int(fx + fw / 2), fy + fh / 2
    rx = fw * .5 + 12
    radii = np.array([rx, rx * 1.22, max(fh * .56, rx * .70)])
    center = np.array([float(cx), -ground_y / SIN, (ground_y - cy) / COS])
    ray = np.asarray(RAY)
    if center[2] - radii[2] < 50:
        center += ray * ((50 + radii[2] - center[2]) / SIN)
    clumps=None
    if branch_clumps:
        from crown_clumps import CrownClumps
        clumps=CrownClumps(packet,center,SIN,COS,RAY)
    a = np.sum((ray / radii) ** 2)
    vertices, faces, uvs, slots, known = [], [], [], [], []
    seen = set()
    uv_layer = mesh.uv_layers['Foliage UV']
    for face in mesh.polygons:
        coords = [tuple(uv_layer.data[i].uv) for i in face.loop_indices]
        observed_skin = face.material_index in (0, 2)
        if observed_skin:
            # The earlier volume builder repeats this exact source tile three
            # times at different depths. A curved skin needs only one copy.
            key = (face.material_index, tuple(coords))
            if key in seen:
                continue
            seen.add(key)
        original = [np.asarray(obj.matrix_world @ mesh.vertices[mesh.loops[loop].vertex_index].co)
                    for loop in face.loop_indices]
        patches = [(original, coords)]
        if observed_skin:
            # Small independent fragments sample the curved envelope without
            # stretching source leaves across steeply sloping shell faces.
            n = 4
            def sample(i, j):
                weights = np.array([1-(i+j)/n, i/n, j/n])
                return (sum(weights[k]*original[k] for k in range(3)),
                        sum(weights[k]*np.asarray(coords[k]) for k in range(3)))
            patches = []
            for i in range(n):
                for j in range(n-i):
                    indices = [(i,j),(i+1,j),(i,j+1)]
                    triangles = [indices]
                    if i+j < n-1:
                        triangles.append([(i+1,j),(i+1,j+1),(i,j+1)])
                    for indices in triangles:
                        samples = [sample(*index) for index in indices]
                        patches.append(([v[0] for v in samples],[v[1] for v in samples]))
        for points, patch_uv in patches:
            start = len(vertices)
            depth = None
            if observed_skin:
                average = np.mean(points,axis=0)
                x,y = average[0], -average[1]*SIN-average[2]*COS
                relative = np.array([x-cx,-(y-cy)*SIN,-(y-cy)*COS])
                b = 2*np.sum(relative*ray/radii**2)
                c = np.sum((relative/radii)**2)-1
                depth = (-b+math.sqrt(max(0.,b*b-4*a*c)))/(2*a)
                depth += 5*math.sin(x*.052+y*.031)+3*math.sin(x*.11-y*.057)
                depth += 1.5*math.sin(x*2.731+y*3.237)
                if clumps is not None:
                    depth=clumps.front_depth(x,y)
                if fragment_depth_jitter:
                    # Camera-ray displacement preserves native projected vertices.
                    # Quantization keeps paired front/back fragment offsets equal.
                    qx,qy=round(x,4),round(y,4)
                    hashed=math.sin(qx*12.9898+qy*78.233)*43758.5453
                    depth += fragment_depth_jitter*(2*(hashed-math.floor(hashed))-1)
                if face.material_index == 2:
                    depth -= .02
            for point, uv in zip(points,patch_uv):
                if observed_skin:
                    x,y = point[0], -point[1]*SIN-point[2]*COS
                    relative = np.array([x-cx,-(y-cy)*SIN,-(y-cy)*COS])
                    point = center+relative+ray*depth
                vertices.append(point.tolist())
                uvs.append(tuple(uv))
            faces.append(tuple(range(start,len(vertices))))
            slots.append(face.material_index)
            known.append(face.material_index == 0)
    materials = list(mesh.materials)
    # Fill the visible interior from both sides without reusing source pixels
    # from a different screen position. The paired front atlas uses the native
    # RGB projected at each texel; the existing rear keeps the inferred palette.
    directory = Path(packet['lobes'][0]['image']).parent
    native = np.asarray(Image.open(directory/'complete-source.png').convert('RGBA'))
    atlas = np.asarray(Image.open(directory/'inferred-interior-atlas.png').convert('RGBA')).copy()
    atlas[:,:,:3] = 0
    original_atlas_height=atlas.shape[0]
    x0,y0,sw,sh = packet['native_bbox']
    def source_facing_pair(face):
        points=np.asarray([vertices[i] for i in face])
        normal=np.cross(points[1]-points[0],points[2]-points[0])
        return np.dot(normal,ray)/np.linalg.norm(normal)<-.15
    originals = [(face,slot) for face,slot in zip(list(faces),list(slots))
                 if slot==4 and source_facing_pair(face)]
    # Native alpha is sampled densely enough to avoid thickening the highly
    # fragmented source silhouette when many interior leaves overlap.
    trim_height=int(round(max((1-uvs[i][1])*original_atlas_height for face,slot in originals for i in face)))
    scale=3
    atlas=np.repeat(np.repeat(atlas[:trim_height],scale,axis=0),scale,axis=1)
    ah,aw=atlas.shape[:2]
    for face,slot in originals:
        uv = np.asarray([uvs[i] for i in face])
        pixel = uv*np.array([aw,-original_atlas_height*scale])+np.array([0,original_atlas_height*scale])
        lo=np.maximum(np.floor(pixel.min(axis=0)).astype(int),0)
        hi=np.minimum(np.ceil(pixel.max(axis=0)).astype(int),[aw,ah])
        xx,yy=np.meshgrid(np.arange(lo[0],hi[0])+.5,np.arange(lo[1],hi[1])+.5)
        a2,b2,c2=pixel
        denom=(b2[1]-c2[1])*(a2[0]-c2[0])+(c2[0]-b2[0])*(a2[1]-c2[1])
        if abs(denom)<1e-9:raise ValueError('Degenerate interior leaf UV')
        wa=((b2[1]-c2[1])*(xx-c2[0])+(c2[0]-b2[0])*(yy-c2[1]))/denom
        wb=((c2[1]-a2[1])*(xx-c2[0])+(a2[0]-c2[0])*(yy-c2[1]))/denom
        wc=1-wa-wb
        inside=(wa>=-1e-7)&(wb>=-1e-7)&(wc>=-1e-7)
        points=np.asarray([vertices[i] for i in face])
        world=wa[...,None]*points[0]+wb[...,None]*points[1]+wc[...,None]*points[2]
        sx=np.floor(world[...,0]-x0).astype(int)
        sy=np.floor(-world[...,1]*SIN-world[...,2]*COS-y0).astype(int)
        valid=inside&(sx>=0)&(sx<sw)&(sy>=0)&(sy<sh)
        tile=atlas[lo[1]:hi[1],lo[0]:hi[0]]
        tile[valid,:3]=native[sy[valid],sx[valid],:3]
        tile[inside&~valid,3]=0
        tile[valid,3]=np.minimum(tile[valid,3],native[sy[valid],sx[valid],3])
    atlas_path=directory/'observed-interior-front-atlas.png'
    Image.fromarray(atlas).save(atlas_path)
    front=material(obj.name+' interior projected front leaves',atlas_path,True)
    one_sided(front);front_slot=len(materials);materials.append(front)
    for face,slot in originals:
        start=len(vertices)
        indices=list(reversed(face))
        for i in indices:
            vertices.append((np.asarray(vertices[i])+ray*.01).tolist())
            u,v=uvs[i]
            uvs.append((u,1-(1-v)*original_atlas_height/trim_height))
        faces.append(tuple(range(start,len(vertices))))
        slots.append(front_slot);known.append(True)
    mesh_report = replace_mesh(obj,vertices,faces,uvs,materials,slots,known)
    report['fragment_depth_jitter']=fragment_depth_jitter
    xyz = np.asarray(vertices)
    result = dict(report, **mesh_report)
    result.pop('leaf_clusters', None)
    result['observed_fragment_triangles'] = sum(known)
    result.update(vertices=len(vertices),faces=len(faces),width=float(np.ptp(xyz[:,0])),depth=float(np.ptp(xyz[:,1])),
        geometry_version='microfragment-volume-paired-front-v3',
        method='Small source-facing fragments sample an irregular curved envelope, with paired inferred backs and randomly rotated interior leaf clusters',
        removed_repeated_observed_layers=True, paired_interior_source_fronts=len(originals))
    if clumps is not None:
        result.update(geometry_version='branch-clump-fragments-v1',branch_clumps=clumps.report(),
            method='Native-ray source fragments and hidden leaf clusters follow multiple irregular branch-scale clumps')
    return result
