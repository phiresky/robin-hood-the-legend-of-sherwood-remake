/** Exercise existing display controls against actual loaded compressed textures. */
export async function proveCompressedTextureModes({v,check,wait,digest}) {
  const THREE=window.reviewThree,textures=new Set();
  for(const root of [v.sourceAsset,v.ground,v.objectsRoot])root?.traverse(object=>{
    for(const material of Array.isArray(object.material)?object.material:[object.material]){
      if(material)for(const value of Object.values(material))if(value?.isCompressedTexture)textures.add(value);
    }
  });
  const pinned=await Promise.all([...textures].map(async texture=>({
    texture,source:texture.source,mipmaps:texture.mipmaps,
    levels:await Promise.all(texture.mipmaps.map(async level=>({level,data:level.data,sha256:await digest(level)}))),
  })));
  check('Actual compressed authored mip chains are present',pinned.some(row=>row.levels.length>1),{textures:pinned.length});
  const label=[...document.querySelectorAll('label')].find(row=>row.textContent.trim()==='Smooth textures');
  const input=label?.querySelector('input');check('Actual smooth textures control exists',!!input);
  const receipts=[];
  for(const smooth of [true,false,true]){
    if(input.checked!==smooth)input.click();
    await wait(()=>v.textureDisplay.smooth===smooth,'actual texture display mode');
    let invalid=0;
    for(const row of pinned){
      const t=row.texture,filtered=smooth&&!v.textureDisplay.nearestRequired.has(t);
      const min=filtered?(row.levels.length>1?THREE.LinearMipmapLinearFilter:THREE.LinearFilter):THREE.NearestFilter;
      if(t.generateMipmaps||t.minFilter!==min||t.magFilter!==(filtered?THREE.LinearFilter:THREE.NearestFilter)||t.source!==row.source||t.mipmaps!==row.mipmaps)invalid++;
      for(const saved of row.levels)if(saved.level.data!==saved.data||await digest(saved.level)!==saved.sha256)invalid++;
    }
    check('Compressed data and filters preserved in mode '+receipts.length,invalid===0,{smooth,textures:pinned.length,invalid});
    // Wait for the ordinary viewport render, including any texture upload caused by the UI toggle.
    const frame=v.renderer.info.render.frame;
    await wait(()=>v.renderer.info.render.frame>frame,'ordinary frame after texture toggle');
    const gl=v.renderer.getContext(),error=gl.getError();
    check('Actual GPU accepts compressed mode '+receipts.length,!gl.isContextLost()&&error===gl.NO_ERROR,{smooth,error});
    receipts.push({smooth,textures:pinned.length,renderFrame:v.renderer.info.render.frame,gpuError:error});
  }
  window.compressedTextureModeProof={receipts,textures:pinned.map(row=>({uuid:row.texture.uuid,format:row.texture.format,levels:row.levels.map(({level,sha256})=>({width:level.width,height:level.height,sha256}))}))};
}
