"""Recover exact approved review cameras and original native phase crops."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[4];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
import qualify as q
from PIL import Image
BASE=q.OUT/'approved03-04-textures-v1/private-phase-integration-v1';OUT=BASE/'browser-packet-v1'
def main():
 assert not OUT.exists();OUT.mkdir();exports=json.loads((BASE/'exports-v1/summary.json').read_text());contracts=json.loads((BASE/'mission-contract-delta-v2.json').read_text());rows=[];s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
 for report in exports['phases']:
  record=report['source'];source=Path(record['path']);assert hashlib.sha256(source.read_bytes()).hexdigest()==record['sha256'];bpy.ops.wm.open_mainfile(filepath=str(source));active=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_phase')==record['phase']];bpy.context.view_layer.update();points=np.array([o.matrix_world@v.co for o in active for v in o.data.vertices]);low,high=points.min(0),points.max(0);center=Vector((low+high)/2);scale=float(np.linalg.norm(high-low))*1.12;views=[]
  def matrix(center,direction):
   position=center+direction*4000;m=(center-position).to_track_quat('-Z','Y').to_matrix().to_4x4();m.translation=position;return [list(row)for row in m]
  for i in range(8):views.append({'index':i,'camera_matrix':matrix(center,Vector((c*math.sin(i*math.pi/4),-c*math.cos(i*math.pi/4),s))),'ortho_scale':scale,'width':256,'height':256})
  contract=next(r for r in contracts['occurrences']if r['profile'].endswith(report['profile']));frame=next(r['source_frame']for r in contract['phase_mapping']if r['phase']==record['phase']);xy=frame['top_left'];size=frame['size'];crop=[int(xy[0])-12,int(xy[1])-12,int(xy[0]+size[0])+12,int(xy[1]+size[1])+12];cx=(crop[0]+crop[2])/2;cy=(crop[1]+crop[3])/2;native={'camera_matrix':matrix(Vector((cx,-cy/s,0)),Vector((0,-c,s))),'ortho_scale':crop[2]-crop[0],'width':crop[2]-crop[0],'height':crop[3]-crop[1],'crop':crop};key=report['profile']+'-'+record['phase'];art=q.rgba(frame);im=Image.new('RGBA',(native['width'],native['height']));im.alpha_composite(art,(12,12));im.save(OUT/f'{key}-native-source.png');rows.append({'key':key,'profile':report['profile'],'phase':record['phase'],'model':report['output'],'components':[o.name for o in active],'triangles':sum(g['triangles']for g in report['primitive_guards']),'views':views,'native':native,'native_source':str(OUT/f'{key}-native-source.png'),'approved_review':record['review'],'approved_source':record})
 (OUT/'plan.json').write_text(json.dumps({'status':'ROOT_CAPTURE_PREPARATION','rows':rows,'scope':'Eight approvedphase exports independently; no dependency on01/02. Originalcamera first. Native source pixel comparison records differences, does not silently waive edges or inferred surfaces.'},indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
