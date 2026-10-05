"""Guarded single approved texture bake using the shared four-slot pool."""
import argparse,json,sys,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from bake_reviewed_asset import stage

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
    parser=argparse.ArgumentParser();parser.add_argument('experiment',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);experiment=args.experiment.resolve();manifest=json.loads((experiment/'views.json').read_text());review=json.loads((experiment/'prebake-review.json').read_text());generation=experiment/review['generation'];image=generation/'generated-preserved.png';assert sha(image)==review['preserved_sha256'];assert review['status']=='suitable-for-guarded-bake';output=experiment/'baked-preserved-v1';assert not output.exists()
    acquire();bpy.ops.wm.open_mainfile(filepath=str(experiment/'approved-model.blend'));bpy.context.preferences.filepaths.save_version=0;scene=bpy.data.scenes[manifest['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.world=bpy.data.worlds.new('Croisement03 texture actual-material review');scene.world.color=(.1,.1,.1)
    report=stage(experiment/'views.json',image,output,texels_per_unit=2)
    (output/'inspection-recipe.json').write_text(json.dumps(dict(recipe=str(Path(__file__).resolve()),recipe_sha256=sha(__file__),model_sha256=sha(output/'worker.blend'),actual_sheet_sha256=sha(output/'actual/textured.png'),render=dict(engine='CYCLES',samples=8,transparent_max_bounces=256),native_camera_tile=0,status='Actual eight-view self-review and independent review pending'),indent=2)+'\n');release();print(json.dumps(dict(asset=manifest['asset_id'],geometry_verified=report['geometry_verified'],counts=report['counts'])))
if __name__=='__main__':main()
