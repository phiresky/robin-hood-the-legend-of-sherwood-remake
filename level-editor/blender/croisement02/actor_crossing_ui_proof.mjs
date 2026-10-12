/** Actual editor UI poses with independent source-pixel composition receipts. */
export async function runActorCrossing({v,cfg,check,wait,capture,digest}) {
  const plan=cfg.actorCrossing;
  const {actorCrossingOracle,opaqueBodyOverlap}=await import(plan.oracleModule);
  const read=async path=>(await fetch('/library/'+path)).json();
  const source={name:cfg.entry.mission,data:await read(cfg.entry.mission_data.path),level:await read(cfg.entry.level_data.path),camera:v.bindings.document().camera};
  const binding={identity:'soldiers:0',layer:source.data.soldiers[0].layer,drawHidden:false,outlineColor:0xf800};
  const actorFrame=()=>v.currentActorFrames().editable.find(row=>row.editorId==='import-soldier-0')?.sprite;
  await wait(()=>actorFrame(),'actual imported Archer frame');
  const selection=document.querySelector('[aria-label="Artwork preview character"]');check('Actor selection control offered',!!selection);
  selection.value='import-soldier-0';selection.dispatchEvent(new Event('change',{bubbles:true}));
  const show=document.querySelector('[aria-label="Show character in artwork"]');check('Actor artwork control offered',show);
  await wait(()=>!show.disabled,'actor selection ready');
  if(!show.checked)show.click();await wait(()=>v.actorPreviewStatus().ready,'source-bound actor preview');
  const hidden=document.querySelector('[aria-label="Show hidden character outline"]');check('Hidden actor outline is disabled',hidden&&!hidden.checked);
  document.querySelector('[data-mission-element="import-soldier-0"] button').click();
  await wait(()=>document.querySelector('input[aria-label="X"]'),'actor position controls');
  const originalPosition=[...v.bindings.document().mission.soldiers.find(row=>row.id==='import-soldier-0').position];
  const setPosition=async position=>{
    for(const [i,label]of ['X','Y','Height'].entries()){
      const input=document.querySelector('input[aria-label="'+label+'"]');check('Position control '+label,!!input);
      input.value=String(position[i]);input.dispatchEvent(new Event('change',{bubbles:true}));await new Promise(r=>setTimeout(r,30));
    }
    const a=source.camera.elevation_deg*Math.PI/180;
    await wait(()=>{const p=actorFrame()?.position;return p&&Math.abs(p[0]-position[0])<1e-6&&Math.abs(p[1]-position[2]/Math.cos(a))<1e-6&&Math.abs(p[2]-position[1]/Math.sin(a))<1e-6},'edited actor pose reaches actual frame');
  };
  const composeOwner=v.actorPreview.composition,originalCompose=composeOwner.compose;let captured;
  composeOwner.compose=function(renderer,loop,snapshot){captured={loop,snapshot};return originalCompose.call(this,renderer,loop,snapshot)};
  const results=[];window.nativeEffectCrops??=[];
  try{
    for(const [caseIndex,pose]of plan.poses.entries()){
      await setPosition(pose.position);
      const sprite=actorFrame();
      check('Pinned actual actor pixels '+pose.id,await digest(sprite.frame.pixels)===plan.actorFrame.sha256&&sprite.frame.filename==='Archer01'&&sprite.frame.profile==='Archer'&&sprite.frame.action===3&&sprite.frame.direction===6);
      const images=[];
      for(const presentation of ['original-art','physical-over-art','refined-map']){
        const effects=document.querySelector('[aria-label="3D effects over original artwork"]');
        if(effects.checked!==(presentation!=='original-art'))effects.click();
        if(presentation!=='original-art'){
          await wait(()=>document.querySelector('[aria-label="Refined map background"]'),'actor refined receiver control');
          const receiver=document.querySelector('[aria-label="Refined map background"]');if(receiver.checked!==(presentation==='refined-map'))receiver.click();
        }
        captured=undefined;v.seekNativeArt(pose.tick,cfg.focus);
        const actual=v.nativePreviewPixels();check('Actual actor composition captured '+pose.id+' '+presentation,!!captured&&!v.actorPreviewStatus().error);
        const oracle=actorCrossingOracle(source,captured.loop,actorFrame(),binding);
        let mismatches=0;for(let i=0;i<actual.data.length;i++)if(actual.data[i]!==oracle.pixels.data[i])mismatches++;
        check('Independent full-buffer pixel oracle '+pose.id+' '+presentation,mismatches===0,{mismatches,bytes:actual.data.length});
        const order=v.actorPreviewStatus().receipt.identities,actorIndex=order.indexOf(binding.identity),effectIndex=order.indexOf('map-animation:12');
        check('Background effect precedes actor '+pose.id+' '+presentation,effectIndex>=0&&effectIndex<actorIndex);
        const effect=oracle.ordered.find(row=>row.id==='map-animation:12'),overlap=opaqueBodyOverlap(oracle.actor,effect);
        if(pose.b05_body_overlap>0)check('Nonvacuous B05/body overlap '+pose.id+' '+presentation,overlap>0,{pixels:overlap});
        if(presentation==='original-art'){
          check('Pinned source overlap '+pose.id,overlap===pose.b05_body_overlap);
          check('Pinned ordered mask receipt '+pose.id,JSON.stringify(oracle.receipt.applied)===JSON.stringify(pose.applied));
          const canopy=oracle.ordered.find(row=>row.id==='map-animation:1');check('Pinned source canopy overlap '+pose.id,opaqueBodyOverlap(oracle.actor,canopy)===pose.canopy_body_overlap);
        }
        let wrongBytes=0;
        if(overlap){const wrong=oracle.compose([...oracle.ordered.filter(row=>row!==effect),effect]);for(let i=0;i<actual.data.length;i++)if(actual.data[i]!==wrong.data[i])wrongBytes++;check('Oracle detects reversed background order '+pose.id+' '+presentation,wrongBytes>0,{changedBytes:wrongBytes});}
        const region={x:pose.position[0]-45,y:pose.position[1]-75,width:100,height:100},data=new Uint8ClampedArray(region.width*region.height*4);
        for(let y=0;y<region.height;y++){const start=((region.y-captured.loop.origin[1]+y)*actual.width+region.x-captured.loop.origin[0])*4;data.set(actual.data.subarray(start,start+region.width*4),y*region.width*4);}
        const canvas=document.createElement('canvas');canvas.width=region.width;canvas.height=region.height;canvas.getContext('2d').putImageData(new ImageData(data,region.width,region.height),0,0);
        const row={name:`b05-actor-case${caseIndex}-${presentation}-tick${pose.tick}`,pose,presentation,region,origin:captured.loop.origin,camera:source.camera,buffer:{width:actual.width,height:actual.height,sha256:await digest(actual)},crop_sha256:await digest({data}),png:canvas.toDataURL('image/png')};
        window.nativeEffectCrops.push(row);images.push(row);results.push({case:pose.id,presentation,position:pose.position,tick:pose.tick,physicalTick:[...v.missionStates.players.values()][0].player.tick,sourceFrame:oracle.receipt.sourceFrame,receipt:oracle.receipt,bodyOverlap:overlap,wrongOrderChangedBytes:wrongBytes,mismatches,sha256:row.buffer.sha256});
      }
      const panel=document.createElement('div');panel.style.cssText='position:fixed;inset:0;z-index:99999;background:#222;color:white;padding:12px;font:14px sans-serif';
      const title=document.createElement('p');title.textContent=pose.id+' · actual source-camera crops · 3× nearest-neighbor · explicit editor pose, no movement/support claim';panel.append(title);
      const columns=document.createElement('div');columns.style.cssText='display:flex;gap:8px';panel.append(columns);
      for(const row of images){const col=document.createElement('div'),label=document.createElement('p'),img=document.createElement('img');label.textContent=row.presentation;img.src=row.png;img.style.cssText='width:300px;height:300px;image-rendering:pixelated';col.append(label,img);columns.append(col);await img.decode();}
      document.body.append(panel);await capture(`b05-actor-case${caseIndex}-tick${pose.tick}`);panel.remove();
    }
  }finally{
    composeOwner.compose=originalCompose;
    await setPosition(originalPosition);
    const show=document.querySelector('[aria-label="Show character in artwork"]');if(show.checked)show.click();
    await wait(()=>!v.actorPreviewStatus().ready,'actor preview restoration');
    const effects=document.querySelector('[aria-label="3D effects over original artwork"]');if(!effects.checked)effects.click();
    await wait(()=>document.querySelector('[aria-label="Refined map background"]'),'restore receiver control');const receiver=document.querySelector('[aria-label="Refined map background"]');if(receiver.checked)receiver.click();
  }
  window.actorCrossingResults=results;
  return results;
}
