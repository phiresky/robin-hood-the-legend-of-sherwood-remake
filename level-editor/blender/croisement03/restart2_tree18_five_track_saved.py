"""Save five-track tree union with explicit added-branch first-hit provenance."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree18_union_saved as base
import restart2_tree18_saved_review as review
B=base.B


def main():
    source=B/'tree18-united-wood-saved-v1/worker.blend';cpu=B/'tree18-five-track-cpu-v2/geometry.json';out=B/'tree18-five-track-saved-v1'
    sourceguard=cpu.parent/'known-stone-source-check.json';assert json.loads(sourceguard.read_text())['status']=='PASS_NO_KNOWN_STONE_OVERLAP'
    recipe=json.loads(cpu.read_text());part=next(p for p in recipe['parts']if p['track']=='left_ascending');assert len(recipe['parts'])==5
    base.guard.resources();base.guard.acquire(slots=2)
    try:
        base.guard.resources();assert not out.exists();pins={str(p):base.sha(p)for p in [source,cpu,sourceguard,Path(__file__)]}
        bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;target=scene.objects['Provisional Tree18 united wood'];before=base.bvh(target)
        m=bpy.data.meshes.new('Missing ascending left branch');m.from_pydata(part['vertices'],[],part['faces']);m.update();branch=bpy.data.objects.new('Missing ascending left branch',m);scene.collection.objects.link(branch);addition=base.bvh(branch)
        all_vertices=np.array([list(target.matrix_world@v.co)for v in target.data.vertices]+part['vertices']);projected=np.stack((all_vertices[:,0],-all_vertices[:,1]*base.S-all_vertices[:,2]*base.C),axis=1);lo=np.floor(projected.min(0)).astype(int);hi=np.ceil(projected.max(0)).astype(int)
        probes=[(x+.5,y+.5)for y in range(lo[1]-1,hi[1]+2)for x in range(lo[0]-1,hi[0]+2)]
        probes += [tuple(g['source_xy'])for p in recipe['parts']for g in p['source_endpoint_guards']]
        expected=[base.hit([before,addition],p)for p in probes];old=[base.hit([before],p)for p in probes]
        bpy.context.view_layer.objects.active=target;mod=target.modifiers.new('Source-supported fourth upper stem','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=branch;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(branch,do_unlink=True);target['tree18_track']='united_five_source_tracks'
        bm=bmesh.new();bm.from_mesh(target.data);assert all(e.is_manifold for e in bm.edges);unseen=set(bm.verts);components=0
        while unseen:
            pending=[unseen.pop()];components+=1
            while pending:
                for edge in pending.pop().link_edges:
                    for v in edge.verts:
                        if v in unseen:unseen.remove(v);pending.append(v)
        volume=float(bm.calc_volume());bm.free();assert components==1 and volume>0
        out.mkdir();bpy.context.preferences.filepaths.save_version=0;model=out/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<8*1024**2;bpy.ops.wm.open_mainfile(filepath=str(model));after=base.bvh(bpy.context.scene.objects['Provisional Tree18 united wood'])
        failures=[];added=[];errors=[]
        for xy,previous,wanted in zip(probes,old,expected):
            actual=base.hit([after],xy)
            if (wanted is None)!=(actual is None):failures.append({'xy':xy,'expected':wanted,'actual':actual})
            elif wanted is not None:
                error=abs(wanted-actual);errors.append(error)
                if error>.002:failures.append({'xy':xy,'expected':wanted,'actual':actual})
            if (previous is None)!=(wanted is None)or previous is not None and wanted is not None and abs(previous-wanted)>.002:added.append({'xy':xy,'prior_four_track_distance':previous,'five_track_expected_distance':wanted})
        receipt={'status':'PASS_SAVED_UNION_REQUIRES_VISUAL_CONTACT_REVIEW'if not failures else'HOLD_BOOLEAN_FIRSTHIT_DRIFT','pins':pins,'model_sha256':base.sha(model),'connected_components':components,'positive_volume':volume,'samples':len(probes),'max_boolean_firsthit_error':max(errors,default=0),'failures':failures,'new_branch_changed_rays':added,'limits':['New branch legitimately changes missing source geometry; all changed prior rays are retained explicitly.','Union compared to exact five separate source sweeps, not merely a regenerated guard; all prior four geometry arrays remained unchanged at input.','Exact source pixel assignments and full bank/wood/native ownership remain separate checks.']};(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');assert not failures
        review.F=out;review.O=out/'saved-review-v1';review.main()
        assert all(base.sha(Path(p))==h for p,h in pins.items());assert sum(p.stat().st_size for p in out.rglob('*')if p.is_file())<32*1024**2
    finally:base.guard.release()


if __name__=='__main__':main()
