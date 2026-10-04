"""Keep a finite, source-domain-based work plan beside the exhaustive mask audit."""
from collections import Counter
import json
from pathlib import Path
from audit_native_mask_backlog import OUT,audit,sha

BATCHES=[
    ('northwest-rock-companion',[54],'foliage','Exact native54, two lobes and inferred west completion; joint with revised northwest rocks.'),
    ('isolated-northern-plants',[73,92],'ground-plants','No native overlaps; infer north-edge73 continuation, preserve92 exact native source.'),
    ('central-bank-companions',[67,68,69,70,71,72],'ground-plants','Resolve canopy/wood overlaps, then use actual bank support before freezing review cameras.'),
    ('western-rock-complements',[57,60],'foliage','Use reviewed rock complements351/352, subtract existing foliage source owners, inspect remaining wood before authoring.'),
    ('covered-forest-overlaps',[62,63,64],'foliage','Partition shared62/63 leaves and existing wood; retain patch-state ownership136/137.64 includes existing birch wood.'),
    ('northern-boundary-fragment',[22],'foliage','Leaves, not a new trunk. Resolve association with existing canopy134/135 and infer only defensible off-map volume.'),
    ('southwest-small-plants',[79,80,82],'ground-plants','Leaf-only wood overlaps retained; exclude authored grass116, infer west-edge80 continuation.'),
    ('southwest-thickets',[77,78,83,84],'foliage','Respect fences/logs, grass116 ownership and southwest rock52.83/84 overlap requires explicit shared-leaf partition.'),
    ('eastern-and-southern-companions',[74,75,85,86,87,88,89,90,91],'foliage','Partition existing trees, upright fences430/431, wall/stump/shed source. Complete boundary clumps; no state-mask scenery substitution.'),
    ('mixed-wattle-and-oak-base',[76,93],'fence-source-audit then foliage','Source splits owned by fence worker.76 growth lies over wattle99;93 includes existing oak35 base, not a separate stump.'),
]
OTHER=[22,57,60,62,63,64,74,75,77,78,79,80,82,83,84,85,86,87,88,89,90,91]
DOMAINS={54:417,76:502,93:503,**dict(zip(OTHER,range(480,502))),**dict(zip([73,92,67,68,69,70,71,72],range(470,478)))}


def main():
    ownership=audit(OUT)
    rows={r['native_mask']:r for r in ownership['records']}
    assigned=[i for _,indices,_,_ in BATCHES for i in indices]
    if len(assigned)!=len(set(assigned)):raise ValueError('Duplicate planned source domain')
    pending={i for i,r in rows.items() if r['missing_foliage_domain'] and i<111}
    if not pending<=set(assigned):raise ValueError('Unplanned remaining native foliage: '+str(pending-set(assigned)))
    if len(set(DOMAINS.values()))!=len(DOMAINS):raise ValueError('Reserved authored domain collision')
    records=[]
    for name,indices,owner,reason in BATCHES:
        records.append(dict(batch=name,owner=owner,native_masks=indices,pending_native_masks=[i for i in indices if i in pending],
            reserved_authored_domains={str(i):DOMAINS[i] for i in indices},remaining_reason=reason))
    report=dict(version=1,status='Finite geometry work plan; no source split, model readiness or approval implied',
        catalog_sha256=ownership['inputs']['ownership-revision/catalog.json'],active_groups=ownership['summary']['active_catalog_groups'],
        remaining_non_grass_domains=len(pending),remaining_grass_fern_domains=sum(r['missing_foliage_domain'] and 111<=i<=123 for i,r in rows.items()),
        pending_by_owner=dict(Counter(owner for _,indices,owner,_ in BATCHES for i in indices if i in pending)),batches=records,
        restrictions=['Reserved IDs are proposals until exact reviewed source masks exist.','Ground receiver460 remains separate; next northern batch uses470–477.','Existing authored source ownership and approved assets stay intact; geometry decisions do not imply texture approval.','Native mask count is not a future object count: mixed domains can split, nearby clumps can combine.'])
    target=OUT/'understory-review/remaining-foliage-plan.json';target.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ('batches','restrictions')},indent=2))


if __name__=='__main__':main()
