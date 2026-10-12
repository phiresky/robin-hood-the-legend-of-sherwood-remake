"""Sunburst source/solid packets for Sherwood's fully reprojected worker.

Uses the normal multi-view camera selection, shading and image driver. Unknown
texels come from explicit reprojection provenance, never a gray-color guess.
Canopy alpha controls physical visibility independently of that provenance.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
EDITOR = HERE.parents[1]
sys.path[:0] = [str(HERE), str(EDITOR/'refinement'), str(EDITOR/'refinement/blender'), str(EDITOR/'blender/lincoln')]
import render_slots  # Bind the shared FIFO pool before legacy helpers alter sys.path.
import texture_unseen_fill as uf
import global_reproject as gr
from source_authority import require_validated_source

gr.SCENE, gr.COLLECTION = 'Sherwood Editor Migration', 'Sherwood Working'
uf.ELEVATIONS = (-45, -15, 15, 35, 55, 75)
WORK = EDITOR/'work/needs-review/sherwood-refinement/textures'
ROOT = WORK/'sunburst'
uf.UNSEEN = ROOT
PROVIDER = 'openrouter'
GENERATION = 'generation-short-no-mask-with-lighting-openrouter'
REQUEST = ('actually use the normal texture synthesis system to synthesize proper textures for everything hidden '
           '(after you have done the normal publishing); as in you should do a full reprojection from original art '
           'and then synthesize everything else; The current refinement procedure: Sunburst AI fill from '
           'source-textured and solid view sheets')
MASKS = {}
TARGETS = {}
ACTUAL_TEXTURE_REVIEW = False


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary=path.with_name(path.name+f'.{os.getpid()}.tmp')
    temporary.write_text(json.dumps(data, indent=2)+'\n')
    temporary.replace(path)


class Kind(str):
    def __new__(cls, name, mask, physical):
        obj = str.__new__(cls, name)
        obj.mask, obj.physical = mask, physical
        return obj


def fillable(scene, obj, slot):
    binding = scene.slot_binding(obj, slot)
    if not binding:
        raise ValueError('Untextured receiver: '+obj.name)
    if obj.name not in MASKS:
        raise ValueError('Receiver has no explicit source ownership: '+obj.name)
    physical = bool(binding['material'].get('foliage_physical_opacity'))
    mask = MASKS[obj.name]
    if tuple(binding['image'].size) != mask.shape[::-1]:
        raise ValueError('Ownership dimensions differ: '+obj.name)
    return binding, Kind('ground' if obj.get('source_node') == 'ground' else 'ownership', mask, physical)


def island_unknown(kind, atlas, rows, cols, normals, face_normal, _gr):
    result = kind.mask[rows, cols] == 0
    if kind.physical:
        result &= atlas[rows, cols, 3] >= 128
    return result


class Target(uf.Target):
    def __init__(self, spec, scene, module):
        TARGETS.clear()
        super().__init__(spec, scene, module)
        TARGETS[id(self.corners)] = self

    def shade(self, camera, index, coords, lighting, bvh):
        if ACTUAL_TEXTURE_REVIEW:
            # Actual-texture sheets use only the returned RGB and ownership.
            # Their lighting is already baked into the atlas; skip computing
            # the unused solid-reference shadow channel.
            lighting = {**lighting, 'toward_sun': [0, 0, 0]}
        return super().shade(camera, index, coords, lighting, bvh)

    def atlas(self, image_id):
        if not hasattr(self, '_packet_atlases'):
            self._packet_atlases = {}
        if image_id in self._packet_atlases:
            return self._packet_atlases[image_id]
        rgba = super().atlas(image_id)
        kind = self.image_kind[image_id]
        unknown = (kind.mask == 0) if kind else np.zeros(rgba.shape[:2], bool)
        result = np.concatenate([rgba, unknown[..., None].astype(np.uint8)], axis=2)
        self._packet_atlases[image_id] = result
        return result


def raster(camera, corners, ss):
    """The normal triangle rasterizer with original leaf cutout coverage."""
    target = TARGETS[id(corners)]
    size = camera.tile*ss
    x, y, d = camera.project(corners.reshape(-1, 3))
    xs, ys, d = (x*ss).reshape(-1, 3), (y*ss).reshape(-1, 3), d.reshape(-1, 3)
    depth = np.full((size, size), -np.inf)
    index = np.full((size, size), -1, dtype=np.int64)
    area = (xs[:,1]-xs[:,0])*(ys[:,2]-ys[:,0])-(xs[:,2]-xs[:,0])*(ys[:,1]-ys[:,0])
    x0, x1 = np.maximum(0, np.ceil(xs.min(1)-.5)).astype(int), np.minimum(size-1, np.floor(xs.max(1)-.5)).astype(int)
    y0, y1 = np.maximum(0, np.ceil(ys.min(1)-.5)).astype(int), np.minimum(size-1, np.floor(ys.max(1)-.5)).astype(int)
    candidate = (np.abs(area)>1e-12)&(x1>=x0)&(y1>=y0)
    # Batch small projected triangles. The leaf meshes contain hundreds of
    # thousands of these: allocating NumPy grids once per face dominates review
    # time. Keep each batch bounded, and retain exact depth/first-face ties.
    widths, heights = x1-x0+1, y1-y0+1
    small = candidate & (widths*heights <= 256)
    ts = np.flatnonzero(small)
    sizes = widths[ts]*heights[ts]
    offsets = np.concatenate(([0], np.cumsum(sizes)))
    start = 0
    while start < len(ts):
        end = max(start+1, int(np.searchsorted(offsets, offsets[start]+262144, side='right')-1))
        end = min(end, len(ts))
        counts = sizes[start:end]
        t = np.repeat(ts[start:end], counts)
        local = np.arange(len(t))-np.repeat(np.cumsum(counts)-counts, counts)
        px, py = x0[t]+local%widths[t], y0[t]+local//widths[t]
        gx, gy = px+.5, py+.5
        ax,bx,cx = xs[t].T
        ay,by,cy = ys[t].T
        w0 = ((bx-gx)*(cy-gy)-(cx-gx)*(by-gy))/area[t]
        w1 = ((cx-gx)*(ay-gy)-(ax-gx)*(cy-gy))/area[t]
        w2 = 1-w0-w1
        inside = (w0>=-1e-9)&(w1>=-1e-9)&(w2>=-1e-9)
        for image_id in np.unique(target.tri_image[t]):
            kind = target.image_kind[image_id] if image_id >= 0 else None
            if not kind or not kind.physical:
                continue
            chosen = np.flatnonzero(inside & (target.tri_image[t] == image_id))
            if not len(chosen):
                continue
            uv = (w0[chosen,None]*target.tri_uv[t[chosen],0]
                  +w1[chosen,None]*target.tri_uv[t[chosen],1]
                  +w2[chosen,None]*target.tri_uv[t[chosen],2])
            atlas = target.atlas(image_id)
            h,w = atlas.shape[:2]
            col = np.floor(uv[:,0]*w).astype(int).clip(0,w-1)
            row = np.floor(uv[:,1]*h).astype(int).clip(0,h-1)
            inside[chosen] &= atlas[row,col,3] >= 128
        cells = py[inside]*size+px[inside]
        z = (w0*d[t,0]+w1*d[t,1]+w2*d[t,2])[inside]
        old = depth.ravel()[cells].copy()
        np.maximum.at(depth.ravel(), cells, z)
        # Discard former face IDs only where a nearer surface replaced them.
        index.ravel()[cells[z > old]] = -1
        nearest = z == depth.ravel()[cells]
        first = np.full(size*size, len(corners), np.int64)
        np.minimum.at(first, cells[nearest], t[inside][nearest])
        hit = (first < len(corners)) & ((index.ravel()<0) | (first<index.ravel()))
        index.ravel()[hit] = first[hit]
        start = end
    for t in np.flatnonzero(candidate & ~small):
        gx = np.arange(x0[t],x1[t]+1)+.5
        gy = (np.arange(y0[t],y1[t]+1)+.5)[:,None]
        (ax,bx,cx),(ay,by,cy) = xs[t],ys[t]
        w0 = ((bx-gx)*(cy-gy)-(cx-gx)*(by-gy))/area[t]
        w1 = ((cx-gx)*(ay-gy)-(ax-gx)*(cy-gy))/area[t]
        w2 = 1-w0-w1
        inside = (w0>=-1e-9)&(w1>=-1e-9)&(w2>=-1e-9)
        image_id = target.tri_image[t]
        kind = target.image_kind[image_id] if image_id>=0 else None
        if kind and kind.physical:
            atlas = target.atlas(image_id)
            uv = w0[...,None]*target.tri_uv[t,0]+w1[...,None]*target.tri_uv[t,1]+w2[...,None]*target.tri_uv[t,2]
            h,w = atlas.shape[:2]
            col = np.floor(uv[...,0]*w).astype(int).clip(0,w-1)
            row = np.floor(uv[...,1]*h).astype(int).clip(0,h-1)
            inside &= atlas[row,col,3]>=128
        z = w0*d[t,0]+w1*d[t,1]+w2*d[t,2]
        window = depth[y0[t]:y1[t]+1,x0[t]:x1[t]+1]
        previous = index[y0[t]:y1[t]+1,x0[t]:x1[t]+1]
        take = inside & ((z>window) | ((z==window)&(t<previous)))
        window[take] = z[take]
        index[y0[t]:y1[t]+1,x0[t]:x1[t]+1][take] = t
    return depth,index,(xs,ys)


def bvh_for(target):
    if ACTUAL_TEXTURE_REVIEW:
        return None
    from mathutils.bvhtree import BVHTree
    from physical_opacity import OpacityRegistry
    opacity = OpacityRegistry()
    corners = []
    for record in target.records:
        obj = record['object']
        triangles = {(tuple(t.loops),t.polygon_index):t for t in obj.data.loop_triangles}
        for loops,polygon,points in zip(record['loops'],record['polygons'],record['corners']):
            opacity.add(obj,obj.data,triangles[(tuple(loops),int(polygon))])
            corners.extend(points)
    tree = BVHTree.FromPolygons([tuple(p) for p in corners],
                               [(i,i+1,i+2) for i in range(0,len(corners),3)],all_triangles=True)
    return opacity.wrap(tree)


uf.Target, uf.fillable, uf.island_unknown = Target, fillable, island_unknown
uf.raster, uf.bvh_for = raster, bvh_for
uf.unknown_texels_mask = lambda kind,texels,normals,face_normals,module: texels[:,4]>0


def open_source(source):
    report = json.loads((source/'reprojection.json').read_text())
    require_validated_source(report)
    worker = source/'source-only.blend'
    if uf.sha(worker) != report['worker_sha256']:
        raise ValueError('Source-only worker changed')
    MASKS.clear()
    for name, path in report['ownership'].items():
        if uf.sha(path) != report['ownership_sha256'][name]:
            raise ValueError('Source ownership changed: '+name)
    MASKS.update({name:np.load(path)['ownership'] for name,path in report['ownership'].items()})
    return uf.open_worker(worker)[1], report


def prepare(source, only=None):
    source = Path(source).resolve()
    scene, provenance = open_source(source)
    lighting_path = WORK/'lighting.json'
    lighting = json.loads(lighting_path.read_text())['lighting']
    groups = {}
    for record in scene.meshes:
        obj = record['object']
        groups.setdefault(obj['asset_group'],[]).append(obj.name)
    specs = []
    for asset,names in sorted(groups.items()):
        base={'id':asset,'assets':[asset],'patches':[],'receivers':names}
        if asset != 'sherwood-terrain':
            specs.append(base)
            continue
        # Keep hidden ground detail at a useful scale instead of squeezing the
        # entire 1920-pixel map into a single 512-pixel view.
        for row in range(2):
            for col in range(3):
                specs.append({**base,'id':f'{asset}-{row+1}-{col+1}',
                    'region':{'x':[col*640,(col+1)*640],
                              'y':[-(row+1)*544/math.sin(math.radians(35)),
                                   -row*544/math.sin(math.radians(35))]}})
    write(ROOT/'targets.json',{'targets':specs})
    if only:
        unknown = set(only) - {spec['id'] for spec in specs}
        if unknown:
            raise ValueError('Unknown texture targets: '+', '.join(sorted(unknown)))
        order = {asset:index for index,asset in enumerate(only)}
        specs.sort(key=lambda spec:order.get(spec['id'],len(order)))
    for spec in specs:
        if only and spec['id'] not in only:
            continue
        output = ROOT/spec['id']
        if (output/'preparation.json').exists():
            uf.verify_prepared(output)
            previous=json.loads((output/'views.json').read_text())
            require_validated_source(previous)
            if (previous['worker_sha256'] != provenance['worker_sha256'] or
                    previous['source_reprojection_sha256'] != uf.sha(source/'reprojection.json')):
                raise ValueError('Existing packet belongs to another source revision: '+str(output))
            continue
        target = Target(spec,scene,gr)
        points,normals,interior = target.unknown_texels()
        if not len(points):
            write(output/'no-fill.json',{'unknown_texels':0})
            continue
        frame = points[interior] if interior.any() else points
        views,coverage = uf.select_cameras(target,frame,normals[interior] if interior.any() else normals,
                                          required_views=[(0,35)])
        bvh = bvh_for(target)
        sheets = {name:np.zeros((uf.TILE*2,uf.TILE*4,4),np.uint8) for name in ('input','solid','mask')}
        sheets['mask'][:]=255
        entries=[]
        for i,(azimuth,elevation) in enumerate(views):
            camera = uf.Camera(azimuth,elevation,frame,target.bound(),uf.TILE)
            _,tri,coords = raster(camera,target.corners,uf.SS)
            covered,color,editable,gray = target.shade(camera,tri,coords,lighting,bvh)
            editable &= covered
            tiles = {name:np.zeros((uf.TILE,uf.TILE,4),np.uint8) for name in sheets}
            tiles['mask'][:]=255
            tiles['solid'][covered,:3]=np.rint(gray[covered]).astype(np.uint8)[:,None]
            tiles['solid'][covered,3]=255
            tiles['input'][covered,:3]=np.rint(color[covered]).astype(np.uint8)
            tiles['input'][covered,3]=255
            tiles['input'][editable,:3]=tiles['solid'][editable,:3]
            tiles['mask'][editable,3]=0
            left,top = i%4*uf.TILE,i//4*uf.TILE
            (output/'views').mkdir(parents=True,exist_ok=True)
            for name,tile in tiles.items():
                sheets[name][top:top+uf.TILE,left:left+uf.TILE]=tile
                Image.fromarray(tile).save(output/f'views/view-{i}-{name}.png')
            entries.append(camera.record(i,left,top))
        for name,sheet in sheets.items():
            Image.fromarray(sheet).save(output/(name+'.png'))
        manifest={'version':1,'kind':'sherwood-source-reprojection-fill','asset_id':spec['id'],'target':spec,
                  'worker_sha256':provenance['worker_sha256'],'display_objects':target.digests(),
                  'tile_size':[uf.TILE,uf.TILE],'layout':{'columns':4,'rows':2,'width':uf.TILE*4,'height':uf.TILE*2},
                  'views':entries,'source_reprojection_sha256':uf.sha(source/'reprojection.json'),
                  'source_ownership_validation':provenance['source_ownership_validation'],
                  'lighting':lighting,'lighting_config_sha256':uf.sha(lighting_path),
                  'unknown_texels':len(points),'camera_coverage':coverage,
                  'protected_source_texels':sum(int(np.count_nonzero(kind.mask == 1))
                                                for kind in target.image_kind if kind),
                  'ownership':'Reviewed source masks and first-hit visibility; alpha is physical coverage only'}
        write(output/'views.json',manifest)
        write(output/'authorization.json',{'request':REQUEST,'scope':'Generate hidden-texture candidates after full source reprojection',
                                          'texture_review':'pending','geometry_revision':provenance['geometry_sha256']})
        write(output/'preparation.json',{'files':{str(p.relative_to(output)):uf.sha(p) for p in output.rglob('*') if p.is_file()}})
        print(json.dumps({'prepared':spec['id'],'unknown_texels':len(points),'coverage':coverage}),flush=True)


def generate(only=None, prompt_suffix=''):
    """Run the repository's normal two-image Sunburst driver, four jobs at most."""
    from concurrent.futures import ThreadPoolExecutor
    experiments = [p.parent for p in sorted(ROOT.glob('*/preparation.json'))
                   if not only or p.parent.name in only]
    def run(experiment):
        uf.verify_prepared(experiment)
        manifest=json.loads((experiment/'views.json').read_text())
        require_validated_source(manifest)
        output=experiment/GENERATION
        if (output/'generation.json').exists():
            report=json.loads((output/'generation.json').read_text())
            if report.get('changedProtected') != 0:
                raise ValueError('Protected pixels changed: '+experiment.name)
            return {'asset':experiment.name,'status':'existing'}
        # This records the user's explicit batch-generation request. It does
        # not claim that the user has seen or approved a generated candidate.
        write(experiment/'approval.json',{'status':'approved','approved_by':'user',
              'asset_id':manifest['target']['assets'][0],
              'scope':'candidate generation for the requested full-map reprojection',
              'basis':'explicit batch authorization in this conversation','exact_user_text':REQUEST,
              'geometry_revision':manifest['worker_sha256'],
              'input_sha256':uf.sha(experiment/'input.png'),'texture_approval':'pending'})
        terrain_context = (' This is bare terrain underneath separately modeled objects. Fill missing areas with '
            'continuous earth, fallen leaves, moss, grass, existing paths or river water as appropriate. '
            'Do not reconstruct removed foreground objects in the gray holes: no furniture, fences, buildings, '
            'trunks, containers or other props.') if manifest['target'].get('region') else ''
        command=['node',str(EDITOR/'pipeline/src/refinement/generate-textures.ts'),str(experiment),
                 '--generate','--prompt-variant','short','--no-mask',
                 '--provider',PROVIDER,
                 '--prompt-suffix','This asset is '+manifest['target']['assets'][0].removeprefix('sherwood-').replace('-',' ')+
                 '. Continue its existing materials onto missing surfaces; preserve the approved geometry and do not add objects. '+prompt_suffix+terrain_context,
                 '--lighting-reference',str(experiment/'solid.png')]
        env=dict(os.environ,NODE_USE_ENV_PROXY='1')
        result=subprocess.run(command,cwd=EDITOR.parent,env=env,text=True,capture_output=True)
        (experiment/'generation.log').write_text(result.stdout+result.stderr)
        if result.returncode:
            return {'asset':experiment.name,'status':'failed','log':str(experiment/'generation.log')}
        report=json.loads((output/'generation.json').read_text())
        if report.get('changedProtected') != 0:
            raise ValueError('Protected pixels changed: '+experiment.name)
        return {'asset':experiment.name,'status':'generated','changedProtected':0}
    with ThreadPoolExecutor(max_workers=4) as executor:
        results=[]
        for result in executor.map(run,experiments):
            results.append(result)
            write(ROOT/'generation-batch.json',{'results':results})
            print(json.dumps(result),flush=True)
    if any(row['status']=='failed' for row in results):
        raise RuntimeError('Some Sunburst requests failed; inspect generation-batch.json')


def fill(source, output, only=None):
    """Bake generated candidates, protecting source RGB and every physical alpha."""
    import bpy
    import texture_combine as tc
    from screen_fill import fill_visible_fragments
    source,output=Path(source).resolve(),Path(output).resolve()
    output.mkdir(parents=True,exist_ok=False)
    scene,provenance=open_source(source)
    before_geometry=uf.geometry_record(scene)
    reports=[]
    for experiment in sorted(ROOT.iterdir()):
        if not experiment.is_dir() or (only and experiment.name not in only):
            continue
        if not (experiment/GENERATION/'generation.json').exists():
            continue
        uf.verify_prepared(experiment)
        manifest=json.loads((experiment/'views.json').read_text())
        generation=json.loads((experiment/GENERATION/'generation.json').read_text())
        require_validated_source(manifest)
        if manifest['worker_sha256'] != provenance['worker_sha256']:
            raise ValueError('Texture packet belongs to another source worker: '+experiment.name)
        if generation.get('changedProtected')!=0:
            raise ValueError('Generation modified source pixels')
        target=Target(manifest['target'],scene,gr)
        if target.digests()!=manifest['display_objects']:
            raise ValueError('Packet geometry or UVs changed: '+experiment.name)
        sheet=np.asarray(Image.open(experiment/GENERATION/'generated-preserved.png').convert('RGBA')).astype(float)
        solid=np.asarray(Image.open(experiment/'solid.png'))[...,3]>0
        editable=np.asarray(Image.open(experiment/'mask.png'))[...,3]==0
        cameras=[]
        for entry in manifest['views']:
            camera=uf.Camera(entry['azimuth_degrees'],entry['elevation_degrees'],np.zeros((1,3)),np.zeros((1,3)),uf.TILE)
            camera.matrix=np.array(entry['camera_matrix_world']);camera.rotation=camera.matrix[:3,:3]
            camera.toward=camera.rotation[:,2];camera.scale=entry['ortho_scale']
            camera.left,camera.top=entry['crop']['left'],entry['crop']['top']
            depth,_,_=raster(camera,target.corners,uf.SS)
            cameras.append((camera,depth))
        counts={'unknown':0,'generated':0,'unseen':0,'protected_changed':0,'physical_alpha_changed':0}
        interior_masks={}
        for record in target.receivers:
            obj=record['object']
            for slot in np.unique(record['slots']):
                binding,kind=fillable(scene,obj,int(slot))
                image=binding['image'];atlas=gr.read_image(image);before=atlas.copy()
                wrote = False
                uv=gr.slot_uvs(obj,binding['uv'])
                interiors=np.zeros(atlas.shape[:2],dtype=bool)
                region_interiors=np.zeros(atlas.shape[:2],dtype=bool)
                for _,rr,cc,pp,_,inside in gr.islands(record,uv,image.size,lambda group:record['slots'][group[0]]==slot):
                    interiors[rr[inside],cc[inside]]=True
                    scoped=inside & target.in_region(obj,pp)
                    region_interiors[rr[scoped],cc[scoped]]=True
                interior_masks[image.name]=region_interiors
                for face,rows,cols,positions,normals,inside in gr.islands(record,uv,image.size,lambda group:record['slots'][group[0]]==slot):
                    unknown=island_unknown(kind,atlas,rows,cols,normals,record['face_normals'][face],gr)
                    unknown &= inside | ~interiors[rows,cols]
                    unknown &= target.in_region(obj,positions)
                    rows,cols,positions,normals,inside=rows[unknown],cols[unknown],positions[unknown],normals[unknown],inside[unknown]
                    if not len(rows):continue
                    counts['unknown']+=int(inside.sum())
                    best=np.full(len(rows),-np.inf);colors=np.zeros((len(rows),3),np.uint8)
                    for side in (1,-1):
                        pending=~np.isfinite(best)
                        for camera,depth in cameras:
                            score=side*(normals@camera.toward)
                            candidate=pending&(score>uf.FILL_MIN_COSINE)&(score>best)
                            indices=np.flatnonzero(candidate)
                            if not len(indices):continue
                            seen,x,y=uf.visible(camera,depth,uf.SS,positions[indices])
                            view=type('View',(),{'crop':{'left':camera.left,'top':camera.top,'width':uf.TILE,'height':uf.TILE}})
                            rgb,valid=tc.sample(sheet,solid,view,x+camera.left,y+camera.top,editable,np.ones(len(indices),bool))
                            take=seen&valid;selected=indices[take]
                            colors[selected]=rgb[take];best[selected]=score[selected]
                    filled=np.isfinite(best)
                    wrote |= bool(filled.any())
                    atlas[rows[filled],cols[filled],:3]=colors[filled]
                    kind.mask[rows[filled],cols[filled]]=2
                    counts['generated']+=int((filled&inside).sum())
                    counts['unseen']+=int((~filled&inside).sum())
                protected=kind.mask==1
                if not np.array_equal(before[protected],atlas[protected]):
                    raise ValueError('Source texels changed: '+obj.name)
                if kind.physical and not np.array_equal(before[...,3],atlas[...,3]):
                    raise ValueError('Physical alpha changed: '+obj.name)
                if not wrote:
                    continue
                material = binding['material']
                material['source_ownership_fill'] = 'synthesized'
                material['texture_review_status'] = 'candidate-review-pending'
                sources = json.loads(material.get('generated_sources_json', '[]'))
                evidence = {'sha256': uf.sha(experiment/GENERATION/'generated-preserved.png'),
                            'views_sha256': uf.sha(experiment/'views.json')}
                if evidence not in sources:
                    sources.append(evidence)
                material['generated_sources_json'] = json.dumps(sources, sort_keys=True)
                material['generated_source_sha256'] = evidence['sha256']
                material['generated_camera_manifest'] = str(experiment/'views.json')
                material['generated_input_sha256'] = uf.sha(experiment/'input.png')
                if not kind.physical:
                    image.alpha_mode = 'CHANNEL_PACKED'
                    atlas[kind.mask != 1, 3] = 0
                gr.write_image(image,atlas)
        # Unlike projecting an atlas center across a nearly edge-on face, this
        # pass transfers only actual first-hit raster fragments. Retain those
        # narrow visible edges instead of dropping them at the center-pass floor.
        corrected=fill_visible_fragments(target,cameras,sheet,solid,editable,raster,gr,uf.SS,1e-8)
        counts['screen_corrected_texels']=sum(corrected.values())
        counts['unseen_after_screen_correction']=sum(int(((kind.mask==0)&interior_masks[image.name]&
            ((gr.read_image(image)[...,3]>=128) if kind.physical else True)).sum())
            for image,kind in zip(target.images,target.image_kind) if kind and image.name in interior_masks)
        for record in target.receivers:
            obj=record['object']
            for slot in np.unique(record['slots']):
                binding,kind=fillable(scene,obj,int(slot))
                if binding['image'].name not in corrected:continue
                material=binding['material']
                material['source_ownership_fill']='synthesized'
                material['texture_review_status']='candidate-review-pending'
                evidence={'sha256':uf.sha(experiment/GENERATION/'generated-preserved.png'),
                          'views_sha256':uf.sha(experiment/'views.json')}
                sources=json.loads(material.get('generated_sources_json','[]'))
                if evidence not in sources:sources.append(evidence)
                material['generated_sources_json']=json.dumps(sources,sort_keys=True)
                material['generated_source_sha256']=evidence['sha256']
                material['generated_camera_manifest']=str(experiment/'views.json')
                material['generated_input_sha256']=uf.sha(experiment/'input.png')
                if not kind.physical:binding['image'].alpha_mode='CHANNEL_PACKED'
        reports.append({'asset':experiment.name,'counts':counts,'generation_directory':GENERATION,
                        'generated_sha256':uf.sha(experiment/GENERATION/'generated-preserved.png'),
                        'views_sha256':uf.sha(experiment/'views.json')})
        print(json.dumps(reports[-1]),flush=True)
    if uf.geometry_record(scene)!=before_geometry:
        raise ValueError('Texture fill changed geometry or UVs')
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'candidate.blend'))
    masks={}
    (output/'ownership').mkdir()
    for name,mask in MASKS.items():
        path=output/'ownership'/(hashlib.sha256(name.encode()).hexdigest()[:20]+'.npz')
        np.savez_compressed(path,ownership=mask)
        masks[name]={'path':str(path),'sha256':uf.sha(path)}
    updates=[]
    (output/'atlas-updates').mkdir()
    for record in scene.meshes:
        obj=record['object']
        if not np.any(MASKS[obj.name]==2):continue
        for slot in np.unique(record['slots']):
            binding,kind=fillable(scene,obj,int(slot))
            image=binding['image'];material=binding['material']
            path=output/'atlas-updates'/(hashlib.sha256((obj.name+':'+str(slot)).encode()).hexdigest()[:20]+'.npz')
            np.savez_compressed(path,rgba=gr.read_image(image))
            properties={key:material[key] for key in (
                'source_ownership_fill','texture_review_status','generated_sources_json',
                'generated_source_sha256','generated_camera_manifest','generated_input_sha256') if key in material}
            updates.append({'object':obj.name,'slot':int(slot),'image':image.name,
                'path':str(path),'sha256':uf.sha(path),'alpha_mode':image.alpha_mode,
                'physical_alpha':kind.physical,'material_properties':properties})
    write(output/'fill.json',{'status':'CANDIDATE_REVIEW_PENDING','assets':reports,
                            'source':str(Path(source).resolve()),
                            'worker_sha256':uf.sha(output/'candidate.blend'),
                            'source_worker_sha256':provenance['worker_sha256'],
                            'geometry_and_uvs_unchanged':True,'ownership':masks,'atlas_updates':updates,
                            'ownership_semantics':{'0':'unfilled-or-padding','1':'protected-source','2':'generated'}})


def review(candidate, only=None):
    """Render the actual baked worker in the exact prepared review cameras."""
    global ACTUAL_TEXTURE_REVIEW
    ACTUAL_TEXTURE_REVIEW = True
    candidate = Path(candidate).resolve()
    report = json.loads((candidate/'fill.json').read_text())
    worker = candidate/'candidate.blend'
    if uf.sha(worker) != report['worker_sha256']:
        raise ValueError('Baked worker changed since verification')
    MASKS.clear()
    for name, evidence in report['ownership'].items():
        if uf.sha(evidence['path']) != evidence['sha256']:
            raise ValueError('Baked ownership mask changed: '+name)
        MASKS[name] = np.load(evidence['path'])['ownership']
    _, scene = uf.open_worker(worker)
    for row in report['assets']:
        asset = row['asset']
        if only and asset not in only:
            continue
        receipt = candidate/'renders'/asset/'review.json'
        if receipt.exists():
            prior = json.loads(receipt.read_text())
            if (prior['worker_sha256'] != report['worker_sha256'] or
                    uf.sha(prior['actual_sheet']) != prior['actual_sheet_sha256']):
                raise ValueError('Review evidence changed: '+asset)
            continue
        result = uf.render_actual(asset, scene, gr, candidate)
        result.update(asset=asset, worker_sha256=report['worker_sha256'],
                      views_sha256=uf.sha(ROOT/asset/'views.json'),
                      review_status='pending', atlas_counts=row['counts'])
        write(receipt, result)
        print(json.dumps(result), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source')
    parser.add_argument('--root', default=str(WORK/'sunburst-masked'))
    parser.add_argument('--asset',action='append')
    parser.add_argument('--generate',action='store_true')
    parser.add_argument('--provider',choices=['openai','openrouter'],default='openrouter')
    parser.add_argument('--prompt-suffix',default='')
    parser.add_argument('--fill',metavar='OUTPUT')
    parser.add_argument('--review',metavar='CANDIDATE')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:])
    ROOT=Path(args.root).resolve()
    uf.UNSEEN=ROOT
    PROVIDER=args.provider
    GENERATION='generation-short-no-mask-with-lighting'+('-openrouter' if PROVIDER=='openrouter' else '')
    if args.generate:
        generate(args.asset,args.prompt_suffix)
    elif args.review:
        review(args.review, args.asset)
    elif args.fill:
        if not args.source:parser.error('--source required for fill')
        fill(args.source,args.fill,args.asset)
    else:
        if not args.source:parser.error('--source required for preparation')
        prepare(args.source,args.asset)
