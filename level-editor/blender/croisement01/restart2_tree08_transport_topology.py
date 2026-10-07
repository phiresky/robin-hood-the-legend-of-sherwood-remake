"""Check exact serialized CPU triangle sections before any Blender import."""
import argparse,hashlib,json
from collections import Counter
from pathlib import Path
import numpy as np


def main():
    parser=argparse.ArgumentParser();parser.add_argument('packet',type=Path);args=parser.parse_args()
    packet=args.packet;report=json.loads((packet/'report.json').read_text());mesh_path=packet/'mesh.npz'
    digest=hashlib.sha256(mesh_path.read_bytes()).hexdigest()
    assert digest==report['mesh_sha256'],'CPU mesh changed after source audit'
    mesh=np.load(mesh_path);records=[]
    for section in report['mesh_sections']:
        i=section['index'];vertices=mesh[f'vertices_{i}'];faces=mesh[f'faces_{i}'];triangles=vertices[faces]
        normals=np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0])
        signed_volume=float(np.einsum('ij,ij->i',triangles[:,0]-vertices.mean(0),normals).sum()/6)
        edges=Counter(tuple(sorted((int(face[j]),int(face[(j+1)%3])))) for face in faces for j in range(3))
        directed=Counter((int(face[j]),int(face[(j+1)%3])) for face in faces for j in range(3))
        records.append(dict(trace_id=section['trace_id'],signed_volume=signed_volume,
            zero_area_triangles=int((np.linalg.norm(normals,axis=1)<1e-9).sum()),
            nonmanifold_edges=sum(count!=2 for count in edges.values()),
            same_direction_edges=sum(directed[a,b]!=directed[b,a] for a,b in edges)))
    passed=all(r['signed_volume']>0 and not any(r[k] for k in ['zero_area_triangles','nonmanifold_edges','same_direction_edges']) for r in records)
    result=dict(status='PASS' if passed else 'FAIL',mesh_sha256=digest,sections=records,
        scope='Per-section closed oriented topology only. Overlapping section intersections and saved-model/source/contact proof remain unresolved.')
    destination=packet/'serialized-topology.json'
    with destination.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
    assert passed,result
    print('PASS',len(records),'closed outward sections; no zero-area triangles')

if __name__=='__main__':main()
