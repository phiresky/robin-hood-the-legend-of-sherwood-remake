"""Sample native state pixels lost behind the qualified Tree04 source surfaces."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'joined02-source-roles-v1'

def main():
    assert not DEST.exists();guard_reader(reader,q.ROOT,DEST)
    qualification=json.loads((q.OUT/'qualification.json').read_text())
    evidence=q.OUT/'joined02-v5-holdout-coverage-256'
    render=json.loads((evidence/'report.json').read_text());crop=render['crop']
    row=qualification['rows'][1];bg=Image.open(q.ROOT/qualification['background']['path']).convert('RGBA')
    rays=[]
    for phase,frame in [('initial',row['art']['states']['initial']['frames'][0]),('applied',row['art']['states']['transition']['frames'][-1])]:
        coverage=np.array(Image.open(evidence/f'{phase}-coverage.png'));pixels=np.argwhere(np.all(coverage[:,:,:3]==[255,0,255],axis=2))
        art=q.rgba(frame)
        # Cover the loss region spatially without treating a sparse sample as full proof.
        seen=set()
        for y,x in pixels:
            sx,sy=int(x+crop[0]),int(y+crop[1]);tile=(sx//3,sy//3)
            if tile in seen:continue
            seen.add(tile);rgba=art.getpixel((sx-int(frame['top_left'][0]),sy-int(frame['top_left'][1])))
            rays.append(dict(phase=phase,screen=[sx+.5,sy+.5],source_rgba=rgba,base_rgba=bg.getpixel((sx,sy)),hits=[]))
    textures={}
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        textures[(placed['id'],ni,pi)]=texture_record(mat)
    def finish(rays,records,mp):
        rows=[]
        for ray in rays:
            hit=ray['first_hit'];sample=None
            if hit:
                arr,sampler,factor=textures[(hit['asset'],hit['node_index'],hit['primitive_index'])]
                if arr is not None:
                    u,v=hit['uv'];assert 0<=u<=1 and 0<=v<=1
                    sample=arr[min(arr.shape[0]-1,int(v*arr.shape[0])),min(arr.shape[1]-1,int(u*arr.shape[1]))].tolist()
            rows.append(dict(phase=ray['phase'],screen=ray['screen'],source_rgba=ray['source_rgba'],
                base_rgba=ray['base_rgba'],tree_nearest_rgba=sample,first_tree_hit=hit))
        DEST.mkdir();(DEST/'report.json').write_text(json.dumps(dict(status='SOURCE_ROLE_EVIDENCE_REQUIRES_INTERPRETATION',
            map=q.pin(mp),records=records,coverage=q.pin(evidence/'coverage.json'),rows=rows,
            limits=['Samples one interior loss pixel per3x3native block; not whole-domain geometry proof.',
                    'All role labels require visual/source reasoning; darkening alone is not a shadow classification.',
                    'Native source pixels and static receiver geometry remain unchanged.',
                    'Nearest atlas RGB is diagnostic; runtime filtering may differ.']),indent=2)+'\n')
        print(dict(samples=len(rows),tree_hits=sum(r['first_tree_hit']is not None for r in rows),
            exact_base_samples=sum(r['tree_nearest_rgba']==r['base_rgba']for r in rows)),flush=True)
    reader.main(ray_records=rays,triangle_callback=inspect,postprocess=finish,output=DEST,
                asset_ids={'croisement02-tree-04'},minimum_free_bytes=10*1024**3,
                output_limit_bytes=16*1024**2,expected_map_sha256=qualification['map']['sha256'])

if __name__=='__main__':
    acquire(slots=2)
    try:main()
    finally:release()
