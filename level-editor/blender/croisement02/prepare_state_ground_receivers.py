"""Partition preserved trap-shadow artwork across ground and raised-bank receivers."""
import argparse
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('workspace',type=Path);parser.add_argument('--output',type=Path,default=OUT/'state-ground-receivers-v1');args=parser.parse_args()
    worker=args.workspace.resolve();output=args.output.resolve()
    if output.exists():raise FileExistsError(output)
    output.mkdir(parents=True)
    packet=json.loads((worker/'reference/packet.json').read_text())
    raster=OUT/'terrain-bank-candidate/integration/bank-ground-first-hit.npz'
    if sha(raster)!=packet['bank_first_hit_sha256']:raise ValueError('Frozen ground/bank raster changed')
    data=np.load(raster);owner=data['owner'];names=list(data['names'])
    assemblies_source=OUT/'state-target-evidence/assemblies.json';assemblies_path=output/'assemblies-snapshot.json'
    assemblies_path.write_bytes(assemblies_source.read_bytes());assemblies=json.loads(assemblies_path.read_text())['assemblies']
    preserved=json.loads((worker/'reference/state-source-preservation.json').read_text())['frames'];lookup={(r['patch'],r['state'],r['frame']):r for r in preserved}
    records=[];total=0;unassigned=0
    for assembly in assemblies:
        if assembly['id'] not in ['log-trap','rock-trap']:continue
        for patch in assembly['background_bindings']:
            for state,sequence in patch['states'].items():
                if not sequence:continue
                for index,frame in enumerate(sequence['frames']):
                    source=OUT/'source-states'/frame['image'];row=lookup[(patch['id'],state,index)]
                    if sha(source)!=row['sha256'] or frame['bbox']!=row['bbox']:raise ValueError('State source differs from frozen receiver evidence')
                    alpha=np.asarray(Image.open(source).convert('RGBA'))[:,:,3]>0;x,y,w,h=frame['bbox']
                    labels=owner[y:y+h,x:x+w]
                    if labels.shape!=alpha.shape:raise ValueError('State frame outside map requires explicit clipping')
                    folder=output/patch['id']/f'{state}-{index:03}';folder.mkdir(parents=True)
                    receivers=[];combined=np.zeros_like(alpha)
                    for label in sorted(set(labels[alpha].tolist())):
                        domain=alpha&(labels==label);pixels=int(domain.sum())
                        name=str(names[label]) if label>=0 else 'unresolved'
                        path=folder/f'{name}.png';Image.fromarray(domain.astype('uint8')*255).save(path)
                        if np.any(combined&domain):raise ValueError('Duplicate state pixel ownership')
                        combined|=domain;unassigned+=pixels if label<0 else 0
                        receivers.append(dict(source_node=name,pixels=pixels,domain=str(path),sha256=sha(path)))
                    if not np.array_equal(combined,alpha):raise ValueError('Lost shadow source pixels')
                    total+=int(alpha.sum());records.append(dict(assembly=assembly['id'],mission=patch['mission'],patch=patch['id'],runtime_patch_index=patch['runtime_patch_index'],state=state,frame=index,source=str(source),source_sha256=sha(source),bbox=frame['bbox'],delay=frame['delay'],receivers=receivers,pixels=int(alpha.sum())))
    write_json(output/'receiver-transitions.json',dict(status='source-domain proposal; no applied scene or texture generation',ground_model_sha256=sha(worker/'model.blend'),bank_model_sha256=packet['bank_model_sha256'],first_hit_sha256=sha(raster),assemblies_sha256=sha(assemblies_path),frozen_state_evidence_sha256=sha(worker/'reference/state-source-preservation.json'),frames=records,statistics=dict(frames=len(records),alpha_pixels_counted_per_frame=total,unassigned_pixels=unassigned,lost_pixels=0,duplicate_pixels=0),timing='Not inferred from geometry. Preserve independently reviewed script events: target motion/freeze, metadata obstacle removal, and shadow application are separate events.',limits=['Receiver labels compare native ground with bank0–4 only; final scene/state first-hit requires integration review.','Original RGBA source files are unchanged. Domains partition ownership only; retain source alpha and native compositing when applying shadows.','Visible log/rock/cart target sprite bodies remain separate assets and never become ground texture.']))
    print(len(records),'frames;',total,'alpha samples;',unassigned,'unassigned')


if __name__=='__main__':main()
