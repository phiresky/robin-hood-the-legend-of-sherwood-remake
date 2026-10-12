"""Bounded wood-only endpoint planning; no textures, Blender scene or acceptance claim."""
import json, math, mmap, struct, sys
from pathlib import Path
import numpy as np
from scipy.ndimage import label
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q
S,C=math.sin(math.radians(35)),math.cos(math.radians(35))

def wood(candidate):
    path=q.LIB/candidate['model'];assert q.pin(path)['sha256']==candidate['model_sha256']
    with path.open('rb') as stream,mmap.mmap(stream.fileno(),0,access=mmap.ACCESS_READ) as raw:
        size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);offset=28+size
        def acc(i):
            a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];assert v.get('buffer',0)==0 and 'sparse' not in a
            dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']]);width={'SCALAR':1,'VEC3':3}[a['type']]
            return np.ndarray((a['count'],width),dtype=dt,buffer=raw,offset=offset+v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*width),dt.itemsize))
        t=candidate['placement']['transform'];assert t['rot_deg']==0;world=np.eye(4);world[:3,3]=[t['dx'],t['dz'],t['dy']/S];transforms={}
        def walk(i,parent):
            transforms[i]=parent@q.matrix(doc['nodes'][i])
            for child in doc['nodes'][i].get('children',[]):walk(child,transforms[i])
        for i in doc['scenes'][doc.get('scene',0)]['nodes']:walk(i,world)
        records=[]
        for ni,node in enumerate(doc['nodes']):
            if 'mesh' not in node or ' wood ' not in node.get('name',''):continue
            for pi,p in enumerate(doc['meshes'][node['mesh']]['primitives']):
                assert p.get('mode',4)==4
                mat=doc['materials'][p['material']]
                if mat.get('alphaMode','OPAQUE')!='OPAQUE':continue
                vertices=acc(p['attributes']['POSITION']).copy();vertices=(np.c_[vertices,np.ones(len(vertices))]@transforms[ni].T)[:,:3];tri=vertices[acc(p['indices']).ravel().reshape(-1,3)]
                camera=np.stack([tri[:,:,0],S*tri[:,:,2]-C*tri[:,:,1],C*tri[:,:,2]+S*tri[:,:,1]],axis=2)
                facing=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0])@np.array([0,S,C])
                records.append(dict(asset=candidate['id'],node=node['name'],node_index=ni,primitive_index=pi,material_index=p['material'],tri=tri,camera=camera,lo=camera[:,:,:2].min(1),hi=camera[:,:,:2].max(1),facing=facing))
        return records

def main():
    dest=q.OUT/'profile03-wood-route-plan-v1';assert not dest.exists()
    qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][2]
    receivers=[c for c in row['bounds_candidates'] if c['id'].startswith('croisement02-tree-')]
    primitives=[p for c in receivers for p in wood(c)]
    field=json.loads((q.OUT/'conformation-v5-local-cell-bounds/depth-field.json').read_text())['rows']['3'];anchor=json.loads((q.OUT/'depth-proposal-v1.json').read_text())['3']
    def back(x,y):
        def corner(sx,sy):
            xlo,xhi=row['source_union_box'][0],row['source_union_box'][2];t=(sx-xlo)/(xhi-xlo);wy=anchor['world_y_left']*(1-t)+anchor['world_y_right']*t+anchor['depth_bulge']*math.sin(math.pi*t)
            a,b,c,d=field['box'];advance=field['camera_ray_advance'][sy-b][sx-a] if a<=sx<=c and b<=sy<=d else 0
            return -wy/C-sy*S/C+advance
        return (corner(x,y)+corner(x+1,y+1))*.5-anchor['thickness']
    cache={}
    def query(x,y):
        if (x,y) in cache:return cache[x,y]
        xy=np.array([x+.5,y+.5]);depth=back(x,y);hits=[]
        for p in primitives:
            indices=np.flatnonzero(np.all(p['lo']<=xy,axis=1)&np.all(p['hi']>=xy,axis=1)&(p['facing']>1e-8))
            for ti in indices:
                t=p['camera'][ti];matrix=np.stack([t[1,:2]-t[0,:2],t[2,:2]-t[0,:2]],axis=1)
                if abs(np.linalg.det(matrix))<1e-10:continue
                u,v=np.linalg.solve(matrix,xy-t[0,:2]);weights=np.array([1-u-v,u,v])
                if weights.min() < -1e-7:continue
                hit=float(weights@t[:,2]);gap=depth-hit
                if not 0<gap<=12:continue
                hits.append(dict(screen=xy.tolist(),leaf_back_depth=depth,gap=gap,target=dict(asset=p['asset'],node=p['node'],node_index=p['node_index'],primitive_index=p['primitive_index'],triangle_index=int(ti),material_index=p['material_index'],world_yup=(weights@p['tri'][ti]).tolist(),camera_depth=hit,front_facing_dot=float(p['facing'][ti]),alpha=1.,passes_alpha_and_culling=True,barycentric=weights.tolist())))
        cache[x,y]=sorted(hits,key=lambda h:h['gap']);return cache[x,y]
    phases=[]
    frames=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']
    for phase,f in zip(['initial','transition-0','transition-1','applied'],frames):
        mask=np.array(q.rgba(f))[:,:,3]>0;labs,count=label(mask,np.ones((3,3)));sizes=np.bincount(labs.ravel());components=[];x0,y0=map(int,f['top_left'])
        for component in range(1,count+1):
            if sizes[component]<32:continue
            pixels=np.argwhere(labs==component);xmin,xmax=int(pixels[:,1].min()),int(pixels[:,1].max());candidates=[]
            for y,x in pixels:
                if xmin+16<x<xmax-16:continue
                for hit in query(x0+int(x),y0+int(y))[:1]:candidates.append(dict(hit,side='left' if x<(xmin+xmax)/2 else 'right'))
            selected=[]
            for side in ['left','right']:
                selected.extend(sorted([h for h in candidates if h['side']==side],key=lambda h:h['gap'])[:5])
            components.append(dict(component=component,pixels=int(sizes[component]),candidates=selected))
        phases.append(dict(phase=phase,components=components))
    dest.mkdir();(dest/'report.json').write_text(json.dumps(dict(status='WOOD_ONLY_ROUTE_PLANNING_NOT_CLEARANCE_OR_SUPPORT_ACCEPTANCE',states=phases,receivers=[dict(asset=c['id'],model_sha256=c['model_sha256'])for c in receivers],recipe=q.pin(Path(__file__)),limits=['Reads only opaque wood geometry through mapped buffers; no texture loads or scene imports.','Samples major source components near their left/right ends. Fine isolated leaf details are retained without fabricated twigs.','Other foliage/receivers may intervene. Every proposed finite support needs the full qualified receiver sweep and saved-model/junction review.','No existing geometry, source pixels, masks or state bindings changed.']),indent=2)+'\n')
    print([(r['phase'],[(c['component'],[(h['side'],round(h['gap'],3),h['target']['asset'])for h in c['candidates']])for c in r['components']])for r in phases],flush=True)

if __name__=='__main__':main()
