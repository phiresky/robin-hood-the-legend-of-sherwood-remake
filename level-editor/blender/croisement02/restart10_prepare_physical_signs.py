"""Pin five physical sign instances and conservatively bounded static context."""
import hashlib
import itertools
import json
import math
import struct
from pathlib import Path
import numpy as np

REPO=Path(__file__).resolve().parents[3]
OUT=REPO/'level-editor/work/croisement02-refinement'
ROOT=OUT/'restart10-physical-signs'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def glb_json(p):
    with p.open('rb') as f:
        assert f.read(4)==b'glTF'
        f.read(8)
        n,t=struct.unpack('<II',f.read(8))
        assert t==0x4E4F534A
        return json.loads(f.read(n))


def local_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix']).reshape(4,4).T
    x,y,z,w=node.get('rotation',[0,0,0,1])
    r=np.array([[1-2*y*y-2*z*z,2*x*y-2*z*w,2*x*z+2*y*w],
                [2*x*y+2*z*w,1-2*x*x-2*z*z,2*y*z-2*x*w],
                [2*x*z-2*y*w,2*y*z+2*x*w,1-2*x*x-2*y*y]])
    m=np.eye(4);m[:3,:3]=r@np.diag(node.get('scale',[1,1,1]));m[:3,3]=node.get('translation',[0,0,0])
    return m


def main():
    dest=ROOT/'input-v2';assert not dest.exists() or not any(dest.iterdir());dest.mkdir(parents=True,exist_ok=True)
    sign=OUT/'state-sign-candidate/animation-export-v2/animated-sign.glb'
    assert sha(sign)=='970f8a4c12c34d1a6845686cad31ab6975a3bbcdf390f5f0c65f6ef8a0122e57'
    assembly=OUT/'state-sign-candidate/five-instances-v3/assembly.json'
    instances=json.loads(assembly.read_text())['instances']
    candidate=OUT/'restart2-textures/post-batch15-static-candidate-v6'
    map_path=candidate/'croisement02.rhlos-map.json';scene=json.loads(map_path.read_text())
    assert sha(map_path)=='7722076d8dfc15f9361c2a3f0a7de0d5d144dccd2f04b2e65578df8baa0ead0b'
    sine=math.sin(math.radians(35));cosine=math.cos(math.radians(35))
    contexts=[];all_bounds=[]
    for source in scene['assetSources']+scene['sceneAssets']:
        file=candidate/'map-assets'/source['model'];assert sha(file)==source['model_sha256']
        doc=glb_json(file);placements=[p for p in scene['placements'] if source['id'] in p['assets']]
        if not placements:
            assert source in scene['sceneAssets'];placements=[dict(transform=dict(dx=0,dy=0,dz=0,rot_deg=0),parts={})]
        for placement in placements:
            t=placement['transform'];assert t['rot_deg']==0
            transform=np.eye(4);transform[:3,3]=[t['dx'],t['dz']/cosine,t['dy']/sine]
            if source.get('role')=='ground':
                # The editor adopts the ground under its Z-up map root.
                ground_axis=np.array([[1,0,0,0],[0,0,1,0],[0,-1,0,0],[0,0,0,1]],dtype=float)
                transform=transform@ground_axis
            points=[];hidden={k for k,v in placement.get('parts',{}).items()if v.get('hidden')}
            def visit(index,parent):
                node=doc['nodes'][index]
                if node.get('name') in hidden:return
                m=parent@local_matrix(node)
                if 'mesh'in node:
                    for primitive in doc['meshes'][node['mesh']]['primitives']:
                        a=doc['accessors'][primitive['attributes']['POSITION']]
                        assert 'min'in a and 'max'in a
                        for p in itertools.product(*zip(a['min'],a['max'])):
                            points.append((m@np.array([*p,1]))[:3])
                for child in node.get('children',[]):visit(child,m)
            index=next((i for i,r in enumerate(doc['scenes'])if r.get('name')==source.get('model_scene')),doc.get('scene',0))
            for n in doc['scenes'][index]['nodes']:visit(n,transform)
            if not points:continue
            xyz=np.array(points);xy=np.stack([xyz[:,0],xyz[:,2]*sine-xyz[:,1]*cosine],axis=1)
            bounds=[float(xy[:,0].min()),float(xy[:,1].min()),float(xy[:,0].max()),float(xy[:,1].max())]
            near=[]
            for row in instances:
                nt=row['native_target'];x,y=nt['position_x'],nt['position_y']
                if bounds[0]<x+85 and bounds[2]>x-85 and bounds[1]<y+55 and bounds[3]>y-110:
                    near.append(row['target_index'])
            record=dict(id=source['id'],file=str(file),sha256=source['model_sha256'],
                        source=source,placement=placement,translation=list(transform[:3,3]),world_matrix=transform.T.flatten().tolist(),
                        projected_bounds=bounds,near_targets=near)
            all_bounds.append(record)
            if near:contexts.append(record)
    mission=REPO/'level-editor/library/mission-states/croisement02/source/S03_FoB_MP.rhm.json'
    level=REPO/'level-editor/library/mission-states/croisement02/source/Croisement02.rhp.json'
    native=json.loads(mission.read_text())
    for row in instances:
        assert native['targets'][row['target_index']]==row['native_target']
    sign_doc=glb_json(sign)
    manifest=dict(status='Private physical sign binding inputs; no live changes',
        model=dict(file=str(sign),sha256=sha(sign),clip=sign_doc['animations'][0]['name']),
        mission=dict(file=str(mission),sha256=sha(mission)),level=dict(file=str(level),sha256=sha(level)),
        camera=scene['camera'],instances=[{k:r[k]for k in ['target_index','world_anchor','native_target','mission']}for r in instances],
        context=contexts,candidate_map=dict(file=str(map_path),sha256=sha(map_path)),
        assembly=dict(file=str(assembly),sha256=sha(assembly)),cycle_ticks=64,ticks_per_pose=2,poses=32,
        limits=['No physical animation approval inherited from native-loop presentation.',
                'Context selection is conservative projected mesh bounds; alpha and opposite-view visibility require actual review.',
                'Native display anchors retain support-derived height; action points never replace display positions.',
                'Exact exported geometry and original source materials remain unchanged.'])
    (dest/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    (dest/'all-context-bounds.json').write_text(json.dumps(all_bounds,indent=2)+'\n')
    print('selected context',len(contexts),[(r['id'],r['near_targets'])for r in contexts])


if __name__=='__main__':main()
