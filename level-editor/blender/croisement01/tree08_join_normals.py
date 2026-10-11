"""Localized smooth shading for an inferred Tree08 trunk join; no geometry edits."""
import math
import numpy as np
from scipy.spatial import cKDTree

def apply_join_normals(mesh,vertices):
    ns=np.array([p.normal[:] for p in mesh.vertices]);result=ns.copy()
    sy,cy=math.sin(math.radians(35)),math.cos(math.radians(35));native=-vertices[:,1]*sy-vertices[:,2]*cy
    def smooth(t):
        t=np.clip(t,0,1);return t*t*(3-2*t)
    weight=smooth((native-275)/20)*smooth((365-native)/20)*smooth((vertices[:,0]-510)/15)*smooth((630-vertices[:,0])/15)
    tree=cKDTree(vertices)
    for j in np.flatnonzero(weight):
        ids=np.array(tree.query_ball_point(vertices[j],7.));delta=vertices[ids]-vertices[j]
        w=np.exp(-np.sum(delta*delta,axis=1)/18)*(ns[ids]@ns[j]>.6)
        mean=(ns[ids]*w[:,None]).sum(0);mean/=np.linalg.norm(mean)
        result[j]=ns[j]*(1-weight[j])+mean*weight[j];result[j]/=np.linalg.norm(result[j])
    mesh.normals_split_custom_set_from_vertices(result.tolist());mesh.update()
