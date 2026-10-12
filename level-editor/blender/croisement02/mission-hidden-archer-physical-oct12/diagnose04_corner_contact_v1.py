"""Measure the exact geometry of the one residual shrub contact cell."""
import json,math,mmap,struct,sys
from pathlib import Path
import numpy as np
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q

def clip(poly,planes,eps):
    for plane in planes:
        if not len(poly):break
        result=[];previous=poly[-1];old=previous@plane[:3]+plane[3]
        for current in poly:
            value=current@plane[:3]+plane[3]
            if (old<=eps)!=(value<=eps):result.append(previous+(eps-old)/(value-old)*(current-previous))
            if value<=eps:result.append(current)
            previous,old=current,value
        poly=np.asarray(result).reshape(-1,3)
    return poly

def main():
    qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][3];candidate=next(c for c in row['bounds_candidates']if c['id']=='croisement02-shrub-80');path=q.LIB/candidate['model'];assert q.pin(path)['sha256']==candidate['model_sha256']
    with path.open('rb')as stream,mmap.mmap(stream.fileno(),0,access=mmap.ACCESS_READ)as raw:
        size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);offset=28+size
        def acc(i):
            a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];assert v.get('buffer',0)==0 and 'sparse'not in a
            dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']]);width={'SCALAR':1,'VEC3':3}[a['type']]
            return np.ndarray((a['count'],width),dtype=dt,buffer=raw,offset=offset+v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*width),dt.itemsize))
        s,c=math.sin(math.radians(35)),math.cos(math.radians(35));t=candidate['placement']['transform'];assert t['rot_deg']==0;world=np.eye(4);world[:3,3]=[t['dx'],t['dz'],t['dy']/s];transforms={}
        def walk(i,parent):
            transforms[i]=parent@q.matrix(doc['nodes'][i])
            for child in doc['nodes'][i].get('children',[]):walk(child,transforms[i])
        for i in doc['scenes'][doc.get('scene',0)]['nodes']:walk(i,world)
        node=doc['nodes'][0];primitive=doc['meshes'][node['mesh']]['primitives'][4];indices=acc(primitive['indices']).ravel()[1659*3:1660*3];vertices=acc(primitive['attributes']['POSITION'])[indices].copy();vertices=(np.c_[vertices,np.ones(3)]@transforms[0].T)[:,:3];tri=np.c_[vertices[:,0],s*vertices[:,2]-c*vertices[:,1],c*vertices[:,2]+s*vertices[:,1]]
    field=json.loads((q.OUT/'conformation-v5-local-cell-bounds/depth-field.json').read_text())['rows']['4'];anchor=json.loads((q.OUT/'depth-proposal-v1.json').read_text())['4'];xlo,xhi=row['source_union_box'][0],row['source_union_box'][2]
    def depth(x,y):
        u=(x-xlo)/(xhi-xlo);wy=anchor['world_y_left']*(1-u)+anchor['world_y_right']*u+anchor['depth_bulge']*math.sin(math.pi*u);bx,by,bxx,byy=field['box'];advance=field['camera_ray_advance'][y-by][x-bx]if bx<=x<=bxx and by<=y<=byy else 0
        return -wy/c-y*s/c+advance
    x,y=39,944;d00,d10,d11,d01=[depth(x+dx,y+dy)for dx,dy in [(0,0),(1,0),(1,1),(0,1)]];results=[]
    for dx,dy,diag in [(d10-d00,d11-d10,[-1,1,0,x-y]),(d11-d01,d01-d00,[1,-1,0,y-x])]:
        constant=d00-dx*x-dy*y;planes=np.array([[-1,0,0,x],[1,0,0,-x-1],[0,-1,0,y],[0,1,0,-y-1],diag,[-dx,-dy,1,-constant],[dx,dy,-1,constant-anchor['thickness']]])
        for eps in [1e-8,0,-1e-6]:
            polygon=clip(tri,planes,eps);area=sum(np.linalg.norm(np.cross(polygon[i]-polygon[0],polygon[i+1]-polygon[0]))*.5 for i in range(1,len(polygon)-1))
            results.append(dict(epsilon=eps,vertices=polygon.tolist(),area_3d=float(area),span=[]if not len(polygon)else np.ptp(polygon,axis=0).tolist()))
    output=q.OUT/'geometry-v8-short-support04-v4/residual-corner-contact.json';assert not output.exists();output.write_text(json.dumps(dict(status='TARGETED_GEOMETRY_MEASUREMENT',receiver=q.pin(path),node=0,primitive=4,triangle=1659,source_cell=[x,y],triangle_camera=tri.tolist(),results=results,limits=['Measures only the previously reported residual cell; does not replace full-scene audit.','Alpha was qualified by the prior audit; no texture or receiver changes.']),indent=2)+'\n');print(json.dumps(results))
if __name__=='__main__':main()
