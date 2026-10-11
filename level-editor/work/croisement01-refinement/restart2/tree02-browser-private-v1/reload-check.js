(async () => {
  const expected = window.tree02ReloadExpectation;
  if (!expected) throw Error('Inject the saved pre-reload expectation before running this check');
  const checks = [];
  const check = (name, value, detail) => {
    checks.push({name, pass: !!value, detail});
    if (!value) throw Error(name + ': ' + JSON.stringify(detail));
  };
  const stable = x => Array.isArray(x) ? x.map(stable) : x && typeof x === 'object'
    ? Object.fromEntries(Object.keys(x).sort().map(k => [k, stable(x[k])])) : x;
  const same = (a, b) => JSON.stringify(stable(a)) === JSON.stringify(stable(b));
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const until = async (test, label) => {
    for (let i = 0; i < 2400; i++) { if (await test()) return; await sleep(100); }
    throw Error('Timeout ' + label);
  };
  await until(() => window.reviewViewport?.bindings.document()?.groups.some(g => g.id === expected.insertedGroup), 'normal saved map reload');
  const v = window.reviewViewport;
  const doc = structuredClone(v.bindings.document());
  check('Reload preserves all original and inserted groups', same(doc.groups, expected.groups));
  check('Reload preserves all original and inserted parts', same(doc.objects, expected.objects));
  const inserted = doc.groups.find(g => g.id === expected.insertedGroup);
  check('Reload preserves independently edited X', inserted.transform.dx === expected.insertedX, inserted.transform);
  for (const [id, pin] of Object.entries(expected.sourcePins)) {
    const ref = doc.assetSources.find(x => x.id === id);
    check('Reload exact source pins ' + id, ref?.model_sha256 === pin.model_sha256 && ref.descriptor_sha256 === pin.descriptor_sha256, ref);
  }
  const row = [...document.querySelectorAll('.object-list li.depth-0')].find(r => r.title === (inserted.name ?? inserted.id));
  check('Reloaded inserted group has an actual UI row', !!row);
  row.click();
  await until(() => v.bindings.selection()?.kind === 'group' && v.bindings.selection()?.id === inserted.id, 'reloaded selection');
  window.scatterStage = {name: 'tree02-reloaded-insertion', time: Date.now()};
  await until(() => window.scatterCaptures?.['tree02-reloaded-insertion'], 'reload screenshot');
  return {status: 'RELOAD_EXECUTED_PENDING_VISUAL_REVIEW', checks, document: doc, capture: 'tree02-reloaded-insertion'};
})()
