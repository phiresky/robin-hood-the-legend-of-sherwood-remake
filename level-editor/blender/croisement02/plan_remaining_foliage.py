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
    mixed=OUT/'mixed-wood-audit/east-followup/source-splits.json'
    if mixed.exists():
        split=json.loads(mixed.read_text())
        report['mixed_source_reservations']=dict(evidence=str(mixed),evidence_sha256=sha(mixed),records=[dict(native_mask=r['native'],unassigned_uncertain_pixels=r['uncertain_pixels'],ground_pixels_not_foliage=r['ground_pixels'],existing_mixed_owner_pixels=r['prior_mixed_pixels']) for r in split['records']],meaning='These source-classification reservations remain unresolved even after a leaf complement receives geometry; catalog coverage alone does not settle them.')
    resolved=OUT/'understory-candidates/mixed75-91-source-v2/final-split-status.json'
    newest=OUT/'understory-candidates/mixed75-91-source-v3/final-split-status.json'
    if newest.exists():resolved=newest
    if resolved.exists():
        status=json.loads(resolved.read_text())
        for key in [k for k in ('source_review','receiver_audit','prior_status','revised_fence_partition') if k in status]:
            if sha(Path(status[key]))!=status[key+'_sha256']:raise ValueError('Mixed source classification proof changed')
        report['mixed_source_history']=report.pop('mixed_source_reservations',None)
        report['mixed_source_reservations']=dict(evidence=str(resolved),evidence_sha256=sha(resolved),
            inferred_source_roles_accepted=status['inferred_source_roles_accepted'],geometry_completion=status['geometry_completion'],
            receiver_holds=status['receiver_holds'],meaning='Source boundaries now have explicit reviewed inferred roles. Receiver geometry and ground restoration remain held until their exact coverage proofs pass; leaf catalog registration alone cannot complete these obligations.')
    wood_edges=OUT/'mixed-wood-audit/foliage-splits.json'
    if wood_edges.exists():
        source=json.loads(wood_edges.read_text());edges=[]
        for row in source['records']:
            if sha(Path(row['domain_path']))!=row['domain_sha256']:raise ValueError('Mixed foliage source domain changed')
            edge=Path(row['reserved_edge_path'])
            edges.append(dict(native_mask=row['native_mask'],clear_foliage_domain=row['domain'],reserved_edge_pixels=row['reserved_existing_wood_edge_pixels'],reserved_edge_path=str(edge),reserved_edge_sha256=sha(edge),existing_owner=row['existing_wood_owner'],status='Boundary classification and receiver coverage pending; clear foliage registration does not settle these pixels'))
        report['wattle_oak_boundary_reservations']=dict(evidence=str(wood_edges),evidence_sha256=sha(wood_edges),records=edges,geometry_completion=False)
    target=OUT/'understory-review/remaining-foliage-plan.json';target.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ('batches','restrictions')},indent=2))


if __name__=='__main__':main()
