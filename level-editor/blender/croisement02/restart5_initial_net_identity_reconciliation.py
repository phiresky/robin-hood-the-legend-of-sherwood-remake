"""Bound thin-rope retry avoiding color gains inferred from gray antialias fringe."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
import bake_texture_candidate as guarded
key=sys.argv[sys.argv.index('--')+1];exp=OUT/f'restart5-initial-nets/texture-fill-v1/profile-{key}/experiment';stage=guarded.stage

def exact_protected_reference(manifest,generated,output,**kwargs):
 # The protected sheet is byte-exact at every observed pixel. Using it as the
 # gain reference makes reconciliation identity, avoiding gray fringe tint.
 kwargs['reconciliation_reference']=generated
 return stage(manifest,generated,output,**kwargs)

guarded.stage=exact_protected_reference
out=exp/(sys.argv[sys.argv.index('--')+2] if len(sys.argv)>sys.argv.index('--')+2 else 'bake-v2-identity');result=guarded.run(exp,out,exp/'generation-review-v2.json')
write_json(out/'identity-reconciliation.json',dict(model_sha256=result['candidate_model_sha256'],source_candidate=str(exp/'bake-v1/worker.blend'),source_candidate_sha256=sha(exp/'bake-v1/worker.blend'),protected_reference=str(exp/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-native-alpha-preserved.png'),scope='Same approved geometry, generated sheet, cameras, explicit masks and source preservation. Only low-frequency unknown-color reconciliation is identity; conservative gray fringe is not observed color authority.',reopened_preservation_sha256=sha(out/'reopened-preservation.json')))
