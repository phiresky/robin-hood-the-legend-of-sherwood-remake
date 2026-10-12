"""Locate noncoplanar triangle surface intersections for endpoint contact audits."""
import numpy as np

def intersection_points(vertices_a, face_a, vertices_b, face_b):
    points=[]
    def inside(p,tri):
        a,b,c=tri;u=b-a;v=c-a;w=p-a
        uu=float(u@u);uv=float(u@v);vv=float(v@v);wu=float(w@u);wv=float(w@v);den=uu*vv-uv*uv
        if abs(den)<1e-16:return False
        s=(wu*vv-wv*uv)/den;t=(wv*uu-wu*uv)/den
        return s>=-1e-7 and t>=-1e-7 and s+t<=1+1e-7
    def edges_against(first,second):
        n=np.cross(second[1]-second[0],second[2]-second[0]);norm=float(np.linalg.norm(n))
        if norm<1e-12:return
        n/=norm
        for a,b in zip(first,np.roll(first,-1,axis=0)):
            da=float((a-second[0])@n);db=float((b-second[0])@n)
            if abs(da)<1e-7 and abs(db)<1e-7:
                if inside(a,second):points.append(a)
                if inside(b,second):points.append(b)
                continue
            if da*db>0 or abs(da-db)<1e-12:continue
            p=a+(b-a)*da/(da-db)
            if inside(p,second):points.append(p)
    for i in range(1,len(face_a)-1):
        a=np.asarray([vertices_a[j] for j in [face_a[0],face_a[i],face_a[i+1]]],dtype=float)
        for j in range(1,len(face_b)-1):
            b=np.asarray([vertices_b[k] for k in [face_b[0],face_b[j],face_b[j+1]]],dtype=float)
            edges_against(a,b);edges_against(b,a)
    return [list(p) for p in points]
