"""Independently audit accepted North Tower preview rays against native PNGs."""
import sys,json,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path[:0]=['/usr/lib/python3.14','/usr/lib/python3.14/lib-dynload','/usr/lib/python3.14/site-packages',str(ROOT/'level-editor/blender')]
import bpy,numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from derby_round2_keep_north_review import OUT,NODES
def pixels(path):
    im=bpy.data.images.load(str(path),check_existing=False);w,h=im.size
    a=np.empty(w*h*4,dtype=np.float32);im.pixels.foreach_get(a);bpy.data.images.remove(im)
    return a.reshape(h,w,4)[::-1]
folder=OUT/(sys.argv[sys.argv.index('--packet')+1] if '--packet' in sys.argv else 'approval-final');meta=json.loads((folder/'views.json').read_text())
source=pixels(meta['source_image']);height,width=source.shape[:2]
inventory=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/manifest.json'
records=json.loads(inventory.read_text())['masks'];allowed=np.zeros((height,width),dtype=bool)
for i in [131,140,143]:
    r=records[i];x,y=r['box_top_left'];w,h=r['box_size'];allowed[y:y+h,x:x+w]|=pixels(inventory.parent/r['png'])[:,:,0]>.5
verts=[];faces=[]
for o in bpy.data.collections['Derby Working'].objects:
    if o.type!='MESH' or o.hide_render or o.get('source_node') not in NODES:continue
    offset=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices)
    faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
tree=BVHTree.FromPolygons(verts,faces);down=Vector((0,-math.sin(math.radians(35)),-math.cos(math.radians(35))))
w,h=meta['tile_size'];rows=[]
for r in meta['views']:
    i=r['index'];known=pixels(folder/f'views/view-{i}-known.png')[:,:,0]>.5;color=pixels(folder/f'views/view-{i}-textured.png')
    matrix=Matrix(r['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,-1));scale=r['ortho_scale']/h
    absent=outside=mismatch=0
    for y,x in np.argwhere(known):
        origin=matrix@Vector(((float(x)+.5-w/2)*scale,(h/2-float(y)-.5)*scale,0))
        hit,_,_,_=tree.ray_cast(origin,direction)
        if hit is None:absent+=1;continue
        sx=math.floor(hit.x);sy=math.floor(hit.dot(down))
        if not(0<=sx<width and 0<=sy<height and allowed[sy,sx]):outside+=1
        if np.max(np.abs(color[y,x,:3]-source[sy,sx,:3]))>.002:mismatch+=1
    rows.append({'view':i,'known':int(known.sum()),'no_hit':absent,'outside_reviewed_native_union':outside,'source_rgb_mismatch':mismatch})
report={'views':rows,'known':sum(r['known'] for r in rows),'outside_reviewed_native_union':sum(r['outside_reviewed_native_union'] for r in rows),'source_rgb_mismatch':sum(r['source_rgb_mismatch'] for r in rows),'no_hit':sum(r['no_hit'] for r in rows),'method':'Independent mesh BVH, saved camera rays and raw native PNG union131/140/143; no constraint helper calls. Full-scene source visibility enforced in generation.','foreign_limit':'Native union is a tower envelope, not per-pixel semantic classification; overlapping masks of this same tower are not foreign geometry. No claim that all overlapping native masks must be subtracted.'}
(OUT/'known-pixel-proof.json').write_text(json.dumps(report,indent=2));print(report,flush=True)
