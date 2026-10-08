"""Repair subpixel source-center misses on a private physical wood proposal.

The displacement is bounded, local, and in the source plane. It changes inferred
geometry, never source pixels or retained upper geometry. No Blender IO occurs.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from scipy.ndimage import label, binary_fill_holes
from physical_wood_sections_cpu import projection_coverage, reference_bindings
from prepare_wood_field_integration import source, CONFIG, STUDY
from continuous_wood_field import SIN, COS


def repair(vertices, faces, body, origin, minimum_source_y):
    original = vertices.copy()
    adjustments = []
    for iteration in range(3):
        audit = projection_coverage(vertices, faces, body, origin)
        required = [(x,y) for x,y in audit['missing_coordinates'] if y>=minimum_source_y]
        if not required:
            break
        for x, y in required:
            projected = np.column_stack((vertices[:, 0], -vertices[:, 1]*SIN-vertices[:, 2]*COS))
            target = np.array([x+.5, y+.5])
            starts = projected[faces].reshape(-1, 2)
            ends = projected[np.roll(faces, -1, axis=1)].reshape(-1, 2)
            edges = ends-starts
            t = np.clip(np.sum((target-starts)*edges, axis=1)/np.maximum(np.sum(edges*edges, axis=1), 1e-20), 0, 1)
            nearest = starts+t[:, None]*edges
            distances = np.linalg.norm(nearest-target, axis=1)
            index = int(np.argmin(distances))
            distance = float(distances[index])
            if distance > .5 or distance < 1e-10:
                raise ValueError(f'Not a bounded subpixel repair: {(x,y,distance)}')
            shift = (target-nearest[index])*(distance+.04)/distance
            weights = np.exp(-np.sum((projected-nearest[index])**2, axis=1)/(2*1.5**2))
            # Compact support avoids touching the remote limb cut.
            weights[np.linalg.norm(projected-nearest[index], axis=1)>4.5] = 0
            vertices += weights[:, None]*np.array([shift[0], -shift[1]*SIN, -shift[1]*COS])
            adjustments.append(dict(iteration=iteration, source_pixel=[x,y], boundary_distance=distance, source_shift=shift.tolist(), support_radius=4.5))
    audit = projection_coverage(vertices, faces, body, origin)
    if any(y>=minimum_source_y for x,y in audit['missing_coordinates']):
        raise ValueError('Native source centers remain uncovered')
    drift = float(np.linalg.norm(vertices-original, axis=1).max())
    a = original[faces];b = vertices[faces]
    old = np.cross(a[:,1]-a[:,0], a[:,2]-a[:,0]);new = np.cross(b[:,1]-b[:,0], b[:,2]-b[:,0])
    q = vertices.astype(np.float32)[faces]
    area = np.linalg.norm(np.cross(q[:,1]-q[:,0], q[:,2]-q[:,0]), axis=1)/2
    if drift>.6 or np.any(np.sum(old*new, axis=1)<=0) or np.any(area<1e-9):
        raise ValueError('Displacement or triangle preservation guard failed')
    return vertices, audit, dict(adjustments=adjustments, minimum_source_y=minimum_source_y, scope='Lower source region only; cropped upper ends require retained-limb composition proof', maximum_world_displacement=drift, triangle_indices_unchanged=True, reversed_triangles=0, float32_degenerate_triangles=0)


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--minimum-source-y',type=float,required=True);args=parser.parse_args()
    if args.output.exists():raise FileExistsError(args.output)
    report=json.loads((args.input.parent/'report.json').read_text());tree=report['tree'];box=CONFIG[tree][2]
    observed,path=source(tree,box)
    if hashlib.sha256(path.read_bytes()).hexdigest()!=report['source_sha256']:raise ValueError('Source changed')
    labels,_=label(observed);sizes=np.bincount(labels.ravel());sizes[0]=0;body=binary_fill_holes(labels==sizes.argmax())
    mesh=np.load(args.input,allow_pickle=False)
    vertices,audit,repair_report=repair(mesh['vertices'].copy(),mesh['faces'],body,box[:2],args.minimum_source_y)
    args.output.mkdir(parents=True)
    np.savez_compressed(args.output/'preview-mesh.npz',vertices=vertices,faces=mesh['faces'])
    report.update(source_ground=audit,source_center_repair=repair_report,previous_mesh=dict(path=str(args.input.resolve()),sha256=hashlib.sha256(args.input.read_bytes()).hexdigest()))
    report['permitted_reference_evidence']=reference_bindings()
    if tree==32:
        domain_review=STUDY/'tree32-nine-pixel-domain-proposal/root-review.json'
        digest=hashlib.sha256(domain_review.read_bytes()).hexdigest()
        if digest!='9d5890d66c7f9f67e3f39e93740072cf47adbbd2228b398a4d640fe9bb7949ba':raise ValueError('Accepted nine-pixel domain correction changed')
        report['domain_review']=dict(path=str(domain_review),sha256=digest)
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(source_ground=audit,repair=repair_report),indent=2))


if __name__=='__main__':main()
