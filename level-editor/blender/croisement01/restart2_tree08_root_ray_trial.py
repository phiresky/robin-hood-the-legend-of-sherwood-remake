"""CPU source-ray lower-root hypothesis; frozen upper bark and receiver meshes."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.interpolate import PchipInterpolator
from restart2_tree08_junction_proof import native_depth
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';out=R/'tree08-root-ray-cpu-v2';out.mkdir(exist_ok=True);origin=np.array([552.,-672.,235.]);s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0,-c,s]);source=R/'tree08-v12-chain-cpu-v3';audit=json.loads((source/'report.json').read_text());archive=np.load(source/'mesh.npz');plan=json.loads((source/'fork-union-plan.json').read_text());used={i for g in plan['groups'] for i in g};sections=[]
for name in ['tree08-v12-remaining-group0-stitched-v3-conformed-stable-depth-corrected','tree08-v12-remaining-group1-stitched-v2-stable']:
 a=np.load(R/name/'candidate.npz');sections.append((a['vertices'],a['faces']))
for entry in audit['mesh_sections']:
 i=entry['index']
 if i not in used:sections.append((archive[f'vertices_{i}'],archive[f'faces_{i}']))
vertices=[];faces=[]
for v,f in sections:faces.extend((f+len(vertices)).tolist());vertices.extend(((v-origin).astype(np.float32).astype(float)+origin).tolist())
v=np.array(vertices);f=np.array(faces);native_y=-v[:,1]*s-v[:,2]*c;knots=[[320,0],[340,50],[369,75],[406,75],[439,65],[469,43.5],[485,40]];curve=PchipInterpolator(*np.array(knots).T);delta=np.where(native_y<=320,0,curve(np.clip(native_y,320,485)));candidate=v+delta[:,None]*ray;terminal_extension=.4*np.clip((native_y-469)/.1,0,1);candidate+=terminal_extension[:,None]*np.array([0,-s,-c]);candidate=(candidate-origin).astype(np.float32).astype(float)+origin;before=native_depth([(v,f)]);after=native_depth([(candidate,f)]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;common=np.isfinite(before)&np.isfinite(after);known_error=float(abs(before[core]-after[core]).max());assert known_error<=.0002;assert np.isfinite(after[core]).all();lost=np.isfinite(before)&~np.isfinite(after);gained=~np.isfinite(before)&np.isfinite(after);print('LOSS',np.argwhere(lost).tolist(),'GAIN',np.argwhere(gained).tolist(),flush=True);assert not lost.any();gy,gx=np.where(gained);assert all(574<=x+331<=578 and y+11==469 for y,x in zip(gy,gx));project=lambda p:np.column_stack((p[:,0],-p[:,1]*s-p[:,2]*c));assert np.max(abs(project(candidate)-project(v)))<.46
field=np.load(R/'tree08-receiver-depth-cpu-v1/native-depth.npz');support=np.maximum(field['ground'],field['terrace']);root=(np.indices(before.shape)[0]+11>=340)&np.isfinite(before);remaining=root&(support>after+1e-4);payload=out/'candidate.npz';np.savez_compressed(payload,vertices=candidate,faces=f,before_vertices=v)
record=dict(status='CPU_RAY_DEPTH_HYPOTHESIS_NOT_APPROVED',model_parent_sha256=hashlib.sha256((R/'tree08-wood-prototype-v12-local-junctions/model.blend').read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256(payload.read_bytes()).hexdigest(),knots_native_y_ray_delta=knots,changed_vertices=int(np.count_nonzero(delta)),source_core_pixels=6276,source_core_depth_error=known_error,source_core_position_fixed=bool(np.array_equal(v[native_y<=320],candidate[native_y<=320])),silhouette_lost=int(lost.sum()),silhouette_gained=int(gained.sum()),root_geometry_pixels=int(root.sum()),remaining_receiver_occluded_root_pixels=int(remaining.sum()),known_source_ymax=316,terminal_E_pixel_hit=bool(np.isfinite(after[469-11,578-331])),geometry_approvals='Tree08 lower wood not approved; approved outside objects unchanged',limitations=['Ray-direction depth only, not a new source ownership assignment. All roots remain inferred geometry.','Current terrace cap/slope is an existing approximation, not a measured soil surface.','Terminal inferred trace extended by0.4 native pixel to cover anchorE: only five added silhouette pixels x574..578,y469. This is geometry hypothesis, not a source texture ownership assignment.','Ridge anatomy unchanged. Saved topology, intersection, actual16view and receiver contacts still required.'])
(out/'report.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record,indent=2))
source_img=Image.open(R.parent/'baseline/covered.png').convert('RGB');crop=(485,340,615,478);im=source_img.crop(crop);overlay=im.copy();pixels=overlay.load()
for y in range(340,478):
 for x in range(485,615):
  if y-11>=461:continue
  if np.isfinite(after[y-11,x-331]):pixels[x-485,y-340]=(255,60,100) if remaining[y-11,x-331] else (20,190,190)
sheet=Image.new('RGB',(780,442),'#222');sheet.paste(im.resize((390,414),Image.Resampling.NEAREST),(0,28));sheet.paste(overlay.resize((390,414),Image.Resampling.NEAREST),(390,28));ImageDraw.Draw(sheet).text((4,5),'Native root source | geometry cyan, still receiver-hidden pink (not ownership)',fill='white');sheet.save(out/'root-artwork-coverage.png')
