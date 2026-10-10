// CPU-prepared check for the actual Editor3D fixture; execution belongs to the browser owner.
(async () => {
  const v = window.reviewViewport, T = window.reviewThree, cfg = window.c3CombinedConfig;
  const evidence = window.c3CombinedEvidence = { checks: [], captures: [], ticks: [], owners: [], limitations: [
    'Fixture-owned production SourceAnimationClocks drives the existing SceneryLayer port; this does not prove automatic mission clock wiring.',
    'Pinned models and reachable roles prove structural ownership; pixel first-hit and fern crossings still require visual review.'
  ] };
  const check = (name, pass, detail) => { evidence.checks.push({ name, pass: !!pass, detail }); if (!pass) throw Error(name + ': ' + JSON.stringify(detail)); };
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const until = async (test, name) => { for (let i = 0; i < 2400; i++) { if (await test()) return; await sleep(100); } throw Error('Timeout ' + name); };
  const capture = async name => { await sleep(150); window.scatterStage = { name, time: Date.now() }; await until(() => window.scatterCaptures?.[name], name); evidence.captures.push(name); };
  const hash = async bytes => [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(x => x.toString(16).padStart(2, '0')).join('');
  check('Actual Editor3D and pinned config available', v?.scenery && T && cfg);
  const [{ SourceAnimationClocks }, { sceneryFrameAtTick }] = await Promise.all([import(cfg.clockModule), import(cfg.sceneryModule)]);
  const initial = structuredClone(v.bindings.document());
  evidence.initialDocument = initial;
  const serialized = JSON.stringify(initial);
  const nativeId = 'croisement03-native-arbre06';
  const nativeParts = initial.objects.filter(p => p.node.startsWith('asset:' + nativeId + ':'));
  check('Exactly one native scenery placement part', nativeParts.length === 1, nativeParts);
  for (const [id, pin] of Object.entries(cfg.assets)) {
    const ref = initial.assetSources.find(x => x.id === id);
    check('Pinned loaded document asset ' + id, ref?.model_sha256 === pin.model_sha256 && ref?.descriptor_sha256 === pin.descriptor_sha256, ref);
  }
  await until(() => [...v.scenery.effects.values()].some(e => e.frames?.frames.length === 14 && e.descriptorSha256 === cfg.assets[nativeId].descriptor_sha256), 'native frames');
  const effects = [...v.scenery.effects].filter(([, e]) => e.descriptorSha256 === cfg.assets[nativeId].descriptor_sha256);
  check('Exactly one complete native runtime owner', effects.length === 1, effects.map(([id]) => id));
  const [effectId, effect] = effects[0];
  const frames = effect.frames.frames;
  check('All native frame delays are three', frames.length === 14 && frames.every(f => f.delay === 3));
  check('No competing viewport loop clock', !v.loopClocks.ready);
  const box = new T.Box3();
  for (const n of [12, 13, 14]) {
    const id = 'croisement03-tree-' + n;
    const group = initial.groups.find(g => g.id === id);
    check('Static tree has exactly one group ' + n, initial.groups.filter(g => g.id === id).length === 1);
    const parts = initial.objects.filter(p => p.group === group.id);
    check('No reachable dynamic-frame0 proxy parts ' + n, !parts.some(p => /dynamic-frame0|Arbre06.fragment/i.test(p.node)), parts.map(p => p.node));
    await until(() => { const w = v.groupViews.get(id)?.wrapper; let count = 0; w?.traverse(o => { if (o.isMesh && o.geometry?.attributes?.position) count++; }); return count >= (n === 12 ? 4 : n === 14 ? 3 : 4); }, 'static tree meshes ' + n);
    const wrapper = v.groupViews.get(id).wrapper;
    const meshes = [], nodes = [];
    wrapper.traverse(o => { nodes.push(o.name); if (o.isMesh) meshes.push({ name: o.name, vertices: o.geometry.attributes.position?.count, triangles: (o.geometry.index?.count ?? o.geometry.attributes.position?.count) / 3, visible: o.visible }); });
    check('No loaded dynamic canopy subtree ' + n, !nodes.some(name => /dynamic.frame0|Arbre06.fragment/i.test(name)), nodes);
    const required = n === 13 ? /Local75|static.native.samples|canopy/i : /proposed.static.canopy|static.native.samples/i;
    check('Static foliage subtree present ' + n, nodes.some(name => required.test(name)), nodes);
    wrapper.updateWorldMatrix(true, true); box.union(new T.Box3().setFromObject(wrapper));
    evidence.owners.push({ id, parts: parts.map(p => ({ id: p.id, node: p.node })), nodes, meshes });
  }
  effect.mesh.updateWorldMatrix(true, true); box.union(new T.Box3().setFromObject(effect.mesh));
  const center = box.getCenter(new T.Vector3()), size = box.getSize(new T.Vector3());
  const frameCamera = angle => { const el = initial.camera.elevation_deg * Math.PI / 180; const dir = new T.Vector3(Math.sin(angle) * Math.cos(el), Math.sin(el), Math.cos(angle) * Math.cos(el)); v.setPerspective(0); v.applyState(v.lookState(center.clone().addScaledVector(dir, Math.max(400, size.length() * 2)), center, Math.max(size.x, size.y, size.z) * .7)); };
  const clock = new SourceAnimationClocks();
  const sourceSha256 = await hash(new TextEncoder().encode(JSON.stringify(cfg.sourceAnimation)));
  // No mission is loaded for this map-only fixture. Both context identities bind the exact map.
  clock.reset({ levelSha256: cfg.levelSha256, missionSha256: cfg.levelSha256 });
  const identity = { kind: 'map-animation', index: 5, sourceSha256 };
  const handle = clock.define(identity);
  clock.setState(handle, { active: true, playing: true });
  v.scenery.sourceClocks.bind([{ effectId, descriptorSha256: effect.descriptorSha256, sourceIndex: 5, sourceSha256 }]);
  const sample = () => {
    const cursor = clock.read(handle), index = sceneryFrameAtTick(frames.map(f => f.delay), cursor.tick);
    v.scenery.sourceClocks.sample([{ id: nativeId, source: { kind: 'map-animation', index: 5, sha256: sourceSha256 }, ...cursor, frame: index }]);
    v.scenery.update(performance.now());
    check('Actual scenery sample tick ' + cursor.tick, effect.mesh.visible && !effect.clockError && effect.mesh.geometry === frames[index].geometry && effect.mesh.material.map === frames[index].texture, { tick: cursor.tick, index, error: effect.clockError });
    return { tick: cursor.tick, index, geometry: effect.mesh.geometry.uuid, texture: effect.mesh.material.map.uuid };
  };
  v.select(null); frameCamera(0);
  try {
    evidence.ticks.push(sample());
    await capture('c3-trio-native-initial');
    for (let tick = 1; tick <= 57; tick++) {
      check('Single production clock increment ' + tick, clock.advance(tick, .04) === 1);
      const row = sample(); evidence.ticks.push(row);
      const expected = Math.floor(Math.max(0, tick - 1) % 56 / 4);
      check('Inclusive-delay phase ' + tick, row.index === expected, row);
      if ((tick - 1) % 4 === 0 && tick <= 53) await capture('c3-trio-native-phase-' + String(row.index).padStart(2, '0'));
    }
    check('Four sampled ticks per phase after sentinel', Array.from({ length: 14 }, (_, i) => evidence.ticks.slice(1, 57).filter(r => r.index === i).length).every(n => n === 4));
    check('Complete loop returns frame zero', evidence.ticks.at(-1).index === 0);
    for (const angle of [Math.PI / 4, Math.PI / 2, Math.PI, 3 * Math.PI / 2]) { frameCamera(angle); sample(); await capture('c3-trio-oblique-' + Math.round(angle * 180 / Math.PI)); }
    frameCamera(0); await capture('c3-trio-native-final');
    check('Document unchanged', JSON.stringify(v.bindings.document()) === serialized);
    evidence.status = 'EXECUTED_PENDING_VISUAL_REVIEW';
    return evidence;
  } finally {
    clock.setState(handle, { playing: false });
    // Keep the last real source-clock sample frozen for final inspection.
    evidence.clock = { sourceSha256, sourceIndex: 5, effectId, context: 'map-only', tick: clock.read(handle).tick };
  }
})()
