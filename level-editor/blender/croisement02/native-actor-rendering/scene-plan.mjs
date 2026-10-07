// Orders already-resolved current identities; it does not allocate script handles.
export function planCurrentScene(snapshot) {
  if (!Number.isSafeInteger(snapshot.epoch) || snapshot.epoch < 0 ||
      !Number.isSafeInteger(snapshot.tick) || snapshot.tick < 0 || !Array.isArray(snapshot.elements))
    throw new Error('A single authoritative scene snapshot is required');
  const identities = new Set(), ranks = new Set();
  const background = [], ordered = [], hidden = [];
  for (const element of snapshot.elements) {
    const {identity, creationOrder, displayOrder, active, stage, masking, epoch} = element;
    if (typeof identity !== 'string' || !identity || identities.has(identity))
      throw new Error('Duplicate or missing current source identity');
    if (epoch !== snapshot.epoch) throw new Error(`Retired identity: ${identity}`);
    if (!Number.isSafeInteger(creationOrder) || creationOrder < 0 || ranks.has(creationOrder))
      throw new Error(`Unresolved or duplicate construction rank: ${identity}`);
    if (typeof active !== 'boolean' || !['background','ordered'].includes(stage) ||
        !Number.isFinite(displayOrder) || !Number.isFinite(Math.fround(displayOrder)))
      throw new Error(`Invalid current presentation: ${identity}`);
    if (!['off','character'].includes(masking))
      throw new Error(`Unsupported masking policy: ${identity}`);
    if (stage === 'background' && masking !== 'off')
      throw new Error('Background restoration cannot use character masking');
    if (masking === 'character' && (!Number.isInteger(element.layer) ||
        !Array.isArray(element.mapPosition) || element.mapPosition.length !== 2 ||
        !element.mapPosition.every(Number.isFinite) || typeof element.drawHidden !== 'boolean'))
      throw new Error(`Missing current character mask query: ${identity}`);
    // The private alpha-mask backend deliberately does not implement hidden outlines.
    if (active && masking === 'character' && element.drawHidden)
      throw new Error(`Hidden-outline composition is not implemented: ${identity}`);
    identities.add(identity); ranks.add(creationOrder);
    const row = Object.freeze({...element, displayOrder: Math.fround(displayOrder),
      ...(element.mapPosition ? {mapPosition:Object.freeze([...element.mapPosition])} : {})});
    if (!active) hidden.push(row);
    else (stage === 'background' ? background : ordered).push(row);
  }
  background.sort((a,b)=>a.creationOrder-b.creationOrder);
  ordered.sort((a,b)=>a.displayOrder-b.displayOrder || a.creationOrder-b.creationOrder);
  return Object.freeze({epoch:snapshot.epoch,tick:snapshot.tick,
    background:Object.freeze(background),ordered:Object.freeze(ordered),hidden:Object.freeze(hidden)});
}
