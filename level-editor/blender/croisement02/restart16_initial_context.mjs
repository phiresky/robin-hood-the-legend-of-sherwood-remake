// Add source-backed initial context without adding controls or changing physical families.
export function addInitialContexts(contract, additions) {
  const result=structuredClone(contract);
  const ids=new Set([...(result.native.elements??[]),...(result.native.background_states??[]),...(result.native.patch_states??[])].map(x=>x.id));
  const sources=new Set([...(result.native.elements??[]),...(result.native.background_states??[]),...(result.native.patch_states??[])].filter(x=>x.source?.kind==='mission-patch').map(x=>x.source.index));
  result.native.patch_states??=[];
  for(const source of additions){
    if(ids.has(source.id)||sources.has(source.source.index))throw Error('Duplicate initial patch context');
    if(source.source.kind!=='mission-patch'||!source.initial.length)throw Error('Initial source context is missing');
    const patch=structuredClone(source);
    if(patch.integrate_in_background)patch.activation='initial-only';
    result.native.patch_states.push(patch);ids.add(patch.id);sources.add(patch.source.index);
  }
  return result;
}
