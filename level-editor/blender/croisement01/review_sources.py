"""Persist the visually inspected source-domain backlog without approval claims."""
import hashlib
import json
from pathlib import Path
from catalog import OUT


def main():
    folder=OUT/'source-survey'
    source=json.loads((folder/'inventory.json').read_text())
    classifications={}
    def assign(ids,kind,note):
        for index in ids:
            if index in classifications:raise ValueError('Duplicate classification')
            classifications[index]=dict(kind=kind,note=note,status='refinement pending')
    assign(range(26),'tree-wood','Visible wood includes leaf overlaps. Trace wood ownership and complete the out-of-map tree; mask silhouette alone is not sufficient.')
    assign(range(26,42),'rock','Rocks may share foreground foliage or duplicate another gameplay layer; separate actual source ownership before projection.')
    assign(range(42,64),'undergrowth','Separate foliage volume from wood and rocks; map-edge instances need completed hidden continuation.')
    assign(range(64,70),'stump','Cut or broken stump with ivy/grass in some masks. Match native cap and wood boundary independently.')
    assign([70],'fallen-branch','One bent woody branch with small upper twig and moss; follow its curved axis rather than a straight log.')
    assign([71],'broken-edge-tree','Large split trunk at east boundary, partly outside map; infer complete rear and crown/branch continuation.')
    assign([72],'root-stump','Low mossy stump on northwest embankment, distinct from terrain below.')
    assign([73],'fallen-twig','Small mossy branch in northern woods; no native obstacle assumed.')
    assign(range(74,88),'grass-and-ferns','Small independent vegetation; distinguish distinct clumps and local terrain support.')
    assign(range(88,93),'terrain-projectile-mask','Native type-4 mask spans terrain and foreground objects; not a new independent visual object.')
    assign(range(93,101),'animated-canopy-domain','Eight broad canopy domains; frame alpha describes animation over static artwork, not total occupancy.')
    assign([101,102],'applied-state-mask','Inactive-until-patch source domain. Current static pixels do not describe applied artwork.')
    assert sorted(classifications)==list(range(103))
    sheets={path.name:hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(folder.glob('masks-*.jpg'))}
    report=dict(source_sha256=source['source_sha256'],sheets=sheets,reviewer='Codex',
                scope='Inspected both labeled occlusion-depth layers, all nine mask/context sheets, full native source and all three volume-footprint sheets.',
                status='Source classification reviewed; no geometry or texture approval implied.',
                domain_classification=[dict(index=i,**classifications[i]) for i in range(103)],
                special_cases=['Masks 34/41 and 45/63 depict the same visual objects on different gameplay layers; avoid duplicate assets.',
                    'Masks 42 and 56 mix foreground foliage with wood and need explicit splitting.',
                    'Patch mask references are layer-local: new layer-0 indices 99/100 resolve to global masks 101/102.',
                    'Mask 25 includes distant forest artwork within a narrow strip; do not project it wholesale onto a nearby trunk.'])
    (folder/'visual-classification.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
