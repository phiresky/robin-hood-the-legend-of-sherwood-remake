/** Executable private fixture using production artwork timing and loaded scene roots.
 * The caller owns the loader, renderer, cameras, model-byte pinning and capture IO.
 * Production StateDelivery still has no moving physical-pose integration.
 */
import { nativeTransientPatchFrame, nativePatchBackgroundFrame } from '../../shared/src/native-state-presentation.ts';
import { createPortcullisPoseAdapter } from './restart45_portcullis_pose_adapter.mjs';

function check(value, message) { if (!value) throw new Error(message); }
function equal(a, b, message) { check(JSON.stringify(a) === JSON.stringify(b), message); }
function transforms(root) {
  const rows=[];
  root.traverse(o=>rows.push({object:o, position:o.position.toArray(), quaternion:o.quaternion.toArray(),
    scale:o.scale.toArray(), visible:o.visible, geometry:o.geometry, material:o.material}));
  return rows;
}
function retained(rows, movingRoot, ignoreRootVisibility=false) {
  for (const row of rows) {
    const o=row.object;
    if (o!==movingRoot) equal(o.position.toArray(),row.position,'Child or gatehouse position changed');
    equal(o.quaternion.toArray(),row.quaternion,'Basis rotation changed');
    equal(o.scale.toArray(),row.scale,'Scale changed');
    if (!(ignoreRootVisibility && o===movingRoot)) check(o.visible===row.visible,'Child or gatehouse visibility changed');
    check(o.geometry===row.geometry && o.material===row.material,'Geometry or material replaced');
  }
}

/**
 * capture({label, view, phase, pose, update, roots}) must render/save the supplied
 * world-Z-up saved camera and return a JSON-serializable image receipt. It must
 * not move roots or change their materials. The gatehouse arrives positioned at
 * its own exported pivot; endpoint roots arrive unplaced (root translation zero).
 */
export async function runPortcullisBrowserFixture({covered, raised, gatehouse,
  approvedMotion, sourceRecord, originYUp, contextViews, capture}) {
  check(typeof capture==='function','Explicit async capture callback required');
  check(covered?.isObject3D && raised?.isObject3D && gatehouse?.isObject3D &&
    new Set([covered,raised,gatehouse]).size===3,'Three distinct loaded roots required');
  equal(covered.position.toArray(),[0,0,0],'Covered root must be unplaced');
  equal(raised.position.toArray(),[0,0,0],'Raised root must be unplaced; do not double-lift');
  check(gatehouse.visible,'Positioned gatehouse context must be visible');
  check(sourceRecord.id==='patch-000' && sourceRecord.record.definitive &&
    sourceRecord.record.integrate_in_background && !sourceRecord.record.end_animation_valid,
    'Wrong native portcullis source record');
  equal(sourceRecord.record.door_indices,[13,14],'Wrong native doors');
  const transition=sourceRecord.rows.find(r=>r.action==='PatchTransition')?.frames;
  const initialFrames=sourceRecord.rows.find(r=>r.action==='PatchInitial')?.frames;
  check(transition?.length===45 && transition.every(r=>r.delay===1) && initialFrames,
    'Exact native 45-frame timing required');
  check(contextViews?.length===8 && contextViews.every((r,i)=>r.index===i),
    'Eight ordered saved context cameras required');
  const state={id:'patch-000', definitive:true,integrate_in_background:true,activation:'phases',
    initial:initialFrames,transition,final:[],initial_loop:true,final_loop:false};
  const before=[transforms(covered),transforms(raised),transforms(gatehouse)];
  const rootVisible=[covered.visible,raised.visible];
  const adapter=createPortcullisPoseAdapter(covered,raised,approvedMotion,originYUp);
  const rows=[], images=[], seen=new Set();
  const initial={patch:'patch-000',applied:false,inTransition:false,fxActive:true};
  function verify(pose) {
    const expected=[originYUp[0],originYUp[1]+pose.liftWorldZ,originYUp[2]];
    equal(covered.position.toArray(),expected,'Covered root does not match exact approved pose');
    equal(raised.position.toArray(),originYUp,'Raised root double-lifted or misplaced');
    check(covered.visible===(pose.appearance==='covered') &&
      raised.visible===(pose.appearance==='raised'),'Wrong endpoint visibility');
    retained(before[0],covered,true);retained(before[1],raised,true);retained(before[2],null);
  }
  async function pictures(label,phase,pose,update) {
    // Original art camera must always precede the oblique context view.
    for (const index of [0,2]) {
      const receipt=await capture({label:`${label}-view-${index}`,view:contextViews[index],
        phase,pose,update,roots:{covered,raised,gatehouse}});
      check(receipt!==undefined,'Capture callback did not return a receipt');
      images.push({label:`${label}-view-${index}`,receipt});verify(pose);
    }
  }
  try {
    let pose=adapter.apply(initial);verify(pose);await pictures('initial','initial',pose,0);
    pose=adapter.apply({...initial,inTransition:true,frame:0,counter:65535});verify(pose);
    for (let update=1;update<=92;update++) {
      const tick=update-1, applied=update>=90, frame=Math.min(44,Math.floor(tick/2));
      const snapshot={...initial,applied,inTransition:!applied,fxActive:!applied,frame,counter:tick%2};
      pose=adapter.apply(snapshot);verify(pose);seen.add(pose.pose);
      const sprite=nativeTransientPatchFrame(state,'forward',tick);
      const background=nativePatchBackgroundFrame(state,'forward',tick);
      check(sprite===(applied?undefined:transition[frame]),'Native transient timing mismatch');
      check(background===(applied?transition[44]:undefined),'Native retained background mismatch');
      check(pose.pose===frame,'Wrong physical frame');
      rows.push({update,tick,frame,applied,covered_visible:covered.visible,raised_visible:raised.visible,
        covered_position:covered.position.toArray(),raised_position:raised.position.toArray(),
        transient_visible:!!sprite,retained_background:!!background});
      if (!applied && tick%2===0 && [0,22,35,36,38,44].includes(frame))
        await pictures(`frame-${frame}`,'transition',pose,update);
      if (update===90) await pictures('applied','applied',pose,update);
    }
    check(seen.size===45,'Not all approved poses exercised');
    // Unrelated patch state must not affect this fixture's authoritative input.
    const previous=covered.position.toArray();
    adapter.apply({...initial,applied:true,fxActive:false,winchFrame:44,roomCoverApplied:true});
    equal(covered.position.toArray(),previous,'Independent winch/room influenced gate');
    let rejected=false;
    try {adapter.apply({...initial,patch:'patch-004'});} catch {rejected=true;}
    check(rejected,'Foreign patch unexpectedly accepted');
    equal(covered.position.toArray(),previous,'Rejected input partially moved gate');
    pose=adapter.apply(initial);verify(pose);await pictures('reset','initial',pose,0);
  } finally {
    adapter.dispose();
    retained(before[0],null);retained(before[1],null);retained(before[2],null);
    equal([covered.visible,raised.visible],rootVisible,'Dispose did not restore visibility');
  }
  let disposedRejected=false;
  try {adapter.apply(initial);} catch {disposedRejected=true;}
  check(disposedRejected,'Disposed adapter accepted update');
  return {status:'PASS_PRIVATE_LOADED_POSE_FIXTURE',updates:rows,poses:seen.size,images,
    captures:images.length,disposed_restoration:true,foreign_patch_rejected:true,
    limitations:['Production loader/display and artwork clock can be exercised by caller; the pose adapter is private.',
      'No production physical-pose controller, authoritative engine snapshot transport, door/audio execution or canonical publication established.',
      'Frame36 remains approved inferred motion; existing StateDelivery remains endpoint-only.']};
}
