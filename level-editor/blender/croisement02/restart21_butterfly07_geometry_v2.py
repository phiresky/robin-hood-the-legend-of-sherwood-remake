"""One fixed butterfly07 rest anatomy inferred from its own wide source poses."""
import math
import numpy as np
from scipy.spatial.transform import Rotation
# One global restshape; no per-pose size parameter exists.
WING=np.array([[0.,-2.925,0.],[2.61,-6.435,0.],[6.525,-6.045,0.],
    [6.96,-2.34,0.],[5.365,2.925,0.],[2.32,4.875,0.],[0.,2.145,0.]])
RADII=np.array([.5,2.7,.5])
HINGES=[-.28,.28]
BODY=np.array([[RADII[0]*math.sin(a)*math.cos(t),RADII[1]*math.cos(a),RADII[2]*math.sin(a)*math.sin(t)]
    for a in np.linspace(0,math.pi,9) for t in np.linspace(0,math.tau,17)[:-1]])
def geometry(p):
    rotation=Rotation.from_euler('xyz',p[:3],degrees=True);wings=[]
    for side,(sign,angle) in enumerate([(-1,p[3]),(1,p[4])]):
        points=WING.copy();points[:,0]*=sign
        points=Rotation.from_euler('y',-sign*angle,degrees=True).apply(points)
        points[:,0]+=HINGES[side];wings.append(rotation.apply(points))
    return rotation.apply(BODY),wings
def fixed_geometry():
    return dict(wing_outline=WING.tolist(),body_radii=RADII.tolist(),wing_hinge_offsets=HINGES,
        inferred='One globally fixed revision from butterfly07 own wide phases2,68,69,70,72,73. Wings span13.92 plus hinge gap.56; length11.31. Body length5.4 remains uncertain. No perphase scale.')
