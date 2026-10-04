"""Separate native draw-order dither visibility from permanent canopy geometry."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from native_log_foreground_reference import crop_frame,sha


def main():
    root=OUT/'state-target-evidence/log-trap';reference=root/'native-order-reference'
    frozen_sha=sha(reference/'manifest.json');frozen=json.loads((reference/'manifest.json').read_text())
    target=json.loads((root/'manifest.json').read_text());box=target['bbox'];left,top,right,bottom=box
    wood=np.array(Image.open(root/'tick-089.png'))[:,:,3]>0
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'];m=next(r for r in masks if r['index']==130);mask_path=OUT/'baseline/masks'/m['png'];assert sha(mask_path)==frozen['canopy_source_comparison']['projectile_mask_sha256']
    mx,my=m['box_top_left'];image=Image.open(mask_path).convert('L');mask=np.array(image.crop((left-mx,top-my,right-mx,bottom-my)))>0
    yy,xx=np.mgrid[:bottom-top,:right-left];global_parity=(xx+left+yy+top)%2
    animation=next(r for r in json.loads((OUT/'animation-references/manifest.json').read_text())['animations']if r['index']==2)
    phases=[]
    for phase,frame in enumerate(animation['frames']):
        overlay=np.array(crop_frame(frame,box))[:,:,3]>0
        visible=wood&~overlay
        occupied_parities=[p for p in(0,1)if np.any(overlay&(global_parity==p))]
        assert len(occupied_parities)==1,('native phase does not have a single checkerboard parity',phase)
        empty_parity=1-occupied_parities[0]
        holes=global_parity==empty_parity
        row=dict(phase=phase,frame_sha256=sha(Path(frame['image'])),empty_global_checkerboard_parity=empty_parity,raw_wood=int(wood.sum()),wood_within_occupancy=int((wood&mask).sum()),visible_wood=int(visible.sum()),visible_wood_within_occupancy=int((visible&mask).sum()),visible_within_occupancy_on_checkerboard_holes=int((visible&mask&holes).sum()),visible_within_occupancy_on_drawn_parity=int((visible&mask&~holes).sum()),visible_outside_occupancy=int((visible&~mask).sum()))
        row['checkerboard_hole_fraction_of_visible_within_occupancy']=row['visible_within_occupancy_on_checkerboard_holes']/row['visible_wood_within_occupancy']
        phases.append(row)
        if phase==0:
            assert np.array_equal(visible,np.array(Image.open(reference/'visible-log-phase0.png'))>0)
            phase0_visible=visible;phase0_holes=holes
    joint=OUT/'log-state-foreground-joint-v3';joint_manifest=json.loads((joint/'manifest.json').read_text());scale=joint_manifest['camera']['ortho_scale'];y,x=np.mgrid[:512,:512];ix=np.floor((right-left)/2+(x+.5-256)*scale/512).astype(int);iy=np.floor((bottom-top)/2+(y+.5-256)*scale/512).astype(int);valid=(ix>=0)&(ix<wood.shape[1])&(iy>=0)&(iy<wood.shape[0])
    def project(a):
        result=np.zeros((512,512),bool);result[valid]=a[iy[valid],ix[valid]];return result
    def emission(name):
        pixels=np.array(Image.open(joint/name));return(pixels[:,:,0]>240)&(pixels[:,:,1]<15)&(pixels[:,:,2]>240)
    expected=project(phase0_visible);body=emission('logs-only-visibility.png');actual=emission('joint-visibility.png');overhidden=expected&body&~actual;owned=project(mask);holes=project(phase0_holes)
    categories={'within_occupancy_checkerboard_holes':owned&holes,'within_occupancy_drawn_parity':owned&~holes,'outside_occupancy':~owned}
    current={name:dict(expected_visible=int((expected&domain).sum()),overhidden_by_physical_crowns=int((overhidden&domain).sum()),correctly_visible=int((expected&actual&domain).sum()),missing_body=int((expected&~body&~actual&domain).sum()))for name,domain in categories.items()}
    rgb=np.zeros((wood.shape[0],wood.shape[1],3),np.uint8);rgb[wood&~phase0_visible]=(80,80,80);rgb[phase0_visible&mask&phase0_holes]=(255,190,40);rgb[phase0_visible&mask&~phase0_holes]=(220,50,170);rgb[phase0_visible&~mask]=(50,170,230)
    dest=root/'checkerboard-visibility-audit';dest.mkdir(exist_ok=False);Image.fromarray(rgb).resize((wood.shape[1]*3,wood.shape[0]*3),Image.Resampling.NEAREST).save(dest/'source-categories.png')
    result=dict(status='runtime composition diagnostic; no permanent canopy alpha change proposed',reference_manifest_sha256=frozen_sha,joint_manifest_sha256=sha(joint/'manifest.json'),occupancy_mask_sha256=sha(mask_path),phases=phases,current_physical_joint=current,interpretation=['Target draw replaces static background foliage wherever its own RGB is present; a later visual-animation draw only replaces its occupied checkerboard parity.','The same unoccupied animation samples reveal static foliage before the event and target wood after the target draw. They are composition holes, not evidence that permanent leaves are absent.','A source-faithful state presentation may require a separate appearance/draw-order treatment; this audit does not prove a particular 3D implementation.','Native authored transparent wood holes and static canopy geometry remain distinct constraints. No global alpha punching or tree shape change is justified by this test.'])
    assert sha(reference/'manifest.json')==frozen_sha
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(phase0=phases[0],current_physical_joint=current),indent=2))


if __name__=='__main__':main()
