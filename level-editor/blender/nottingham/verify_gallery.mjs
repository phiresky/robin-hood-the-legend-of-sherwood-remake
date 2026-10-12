// Exercise the actual review page and retain a screenshot and browser report.
import {spawn} from 'node:child_process';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {Temporal} from 'temporal-polyfill';
import {pathToFileURL} from 'node:url';
import {chromeEndpoint, socketOpen, evaluate} from '../../app/tests/cdp.mjs';

const gallery = resolve(process.argv[2] || 'level-editor/work/needs-review/nottingham-refinement/gallery');
const output = resolve(process.argv[3] || 'level-editor/work/needs-review/nottingham-refinement/verification');
await mkdir(output, {recursive:true});
const evidence = JSON.parse(await readFile(join(gallery, 'evidence.json'), 'utf8'));
const profile = join(output, 'browser-profile-' + Temporal.Now.instant().epochMilliseconds);
const chrome = spawn('/usr/lib/chromium/chromium', [
  '--headless', '--no-sandbox', '--disable-dev-shm-usage', '--disable-background-networking',
  '--window-size=1440,1600', '--remote-debugging-port=0', '--user-data-dir=' + profile,
  pathToFileURL(join(gallery, 'index.html')).href,
], {stdio:['ignore','ignore','pipe'], env:{...process.env, TMPDIR:'/tmp'}});
let browserErrors = '';
chrome.stderr.on('data', data => { browserErrors += data.toString(); });
const closed = new Promise(resolve => chrome.on('close', resolve));
let ws;
try {
  const endpoint = new URL(await chromeEndpoint(chrome));
  const pages = await (await fetch('http://' + endpoint.host + '/json/list')).json();
  const page = pages.find(p => p.type === 'page');
  if (!page) throw new Error('Review page not found in browser');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await socketOpen(ws);
  let id = 0;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await evaluate(ws, ++id, "document.readyState === 'complete' && !!document.querySelector('article')")) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  await evaluate(ws, ++id, "document.querySelector('article')?.scrollIntoView()");
  await evaluate(ws, ++id, "document.querySelectorAll('article img').forEach(image => { image.loading = 'eager'; })");
  for (let attempt = 0; attempt < 300; attempt++) {
    if (await evaluate(ws, ++id, "[...document.querySelectorAll('article img')].every(image => image.complete && image.naturalWidth > 0)")) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  const result = await evaluate(ws, ++id, `(() => {
    const cards = [...document.querySelectorAll('article')];
    const firstImages = [...(cards[0]?.querySelectorAll('img') || [])];
    const mode = document.querySelector('#mode');
    mode.value='solid'; mode.dispatchEvent(new Event('change'));
    const solidToggle = document.body.dataset.mode === 'solid';
    mode.value='both'; mode.dispatchEvent(new Event('change'));
    const readiness = document.querySelector('#readiness');
    if (!readiness) throw new Error('Ready-for-review filter is missing');
    readiness.value='ready'; readiness.dispatchEvent(new Event('change'));
    const visibleReady = cards.filter(card => !card.hidden);
    const readyFilter = visibleReady.every(card => card.querySelector('.status').textContent === 'ready-for-user');
    const readyCards = visibleReady.length;
    const readyNavigation = [...document.querySelectorAll('nav a')].filter(link => !link.hidden).length;
    readiness.value='all'; readiness.dispatchEvent(new Event('change'));
    return {title:document.title, cards:cards.length,
      animationStates:document.querySelectorAll('details.animation-state').length,
      separateAnimationCards:cards.filter(card => card.id.includes('--animation-')).length,
      images:document.querySelectorAll('article img').length,
      failedImages:[...document.querySelectorAll('article img')]
        .filter(image => !image.complete || image.naturalWidth === 0)
        .map(image => image.getAttribute('src')),
      readyFilter, readyCards, readyNavigation,
      namedCards:cards.every(card => !!card.querySelector('code')?.textContent),
      reportLinks:cards.every(card => card.querySelectorAll('a[href^="reports/"]').length >= 2),
      navigation:document.querySelectorAll('nav a').length,
      firstImages:firstImages.map(image => ({src:image.getAttribute('src'),
        loaded:image.complete && image.naturalWidth > 0, width:image.naturalWidth,height:image.naturalHeight})),
      solidToggle, horizontalOverflow:document.documentElement.scrollWidth > innerWidth};
  })()`);
  result.expectedCards = evidence.items.length;
  result.expectedReadyCards = evidence.items.filter(item => item.status === 'ready-for-user').length;
  result.expectedAnimationStates = evidence.items.reduce((sum, item) => sum + (item.animation_reviews?.length || 0), 0);
  await evaluate(ws, ++id, `void (async () => {
    const cards = [...document.querySelectorAll('article[data-review-revision]')]
      .filter(card => card.querySelector('.status').textContent === 'ready-for-user');
    if (cards.length < 3) throw new Error('Missing per-asset feedback controls');
    const first = cards[0], second = cards[1];
    const select = first.querySelector('.decision');
    const note = second.querySelector('.review-note');
    select.value = 'approved'; select.dispatchEvent(new Event('change'));
    note.value = 'Fix roof <corner>\\nplease'; note.dispatchEvent(new Event('input'));
    const expected = document.title + '\\n' + first.id + ': approved [review ' + first.dataset.reviewRevision +
      ']\\n' + second.id + ': feedback — Fix roof <corner> please [review ' + second.dataset.reviewRevision + ']';
    const exported = document.querySelector('#review-export').value === expected;
    const saved = JSON.parse(localStorage.getItem('model-review-v1:' + document.title + ':' + first.id + ':' + first.dataset.reviewRevision));
    // Simulate positional native form restoration after cards are removed.
    note.value = 'Feedback restored onto the wrong asset';
    cards[2].querySelector('.decision').value = 'approved';
    cards[2].querySelector('.review-note').value = 'Wrong card';
    window.dispatchEvent(new Event('pageshow'));
    const restoration = note.value === 'Fix roof <corner>\\nplease' &&
      select.value === 'approved' && cards[2].querySelector('.decision').value === '' &&
      cards[2].querySelector('.review-note').value === '' && document.querySelector('#review-export').value === expected;
    let copied = '';
    Object.defineProperty(navigator, 'clipboard', {configurable:true, value:{writeText:async text => { copied = text; }}});
    // Late restoration after pageshow must not contaminate the export either.
    select.value = 'needs refinement';
    note.value = 'Late wrong-card feedback';
    document.querySelector('#copy-reviews').click();
    await new Promise(resolve => setTimeout(resolve, 0));
    const clipboard = copied === expected && select.value === 'approved' && note.value === 'Fix roof <corner>\\nplease';
    const thirdDecision = cards[2].querySelector('.decision');
    const thirdNote = cards[2].querySelector('.review-note');
    thirdDecision.value = 'approved';
    thirdNote.value = 'Deliberate feedback'; thirdNote.dispatchEvent(new Event('input'));
    const noteOnly = document.querySelector('#review-export').value === expected + '\\n' + cards[2].id +
      ': feedback — Deliberate feedback [review ' + cards[2].dataset.reviewRevision + ']';
    thirdNote.value = ''; thirdNote.dispatchEvent(new Event('input'));
    thirdNote.value = 'Restored onto wrong card';
    thirdDecision.value = 'needs refinement'; thirdDecision.dispatchEvent(new Event('change'));
    const decisionOnly = document.querySelector('#review-export').value === expected + '\\n' + cards[2].id +
      ': needs refinement [review ' + cards[2].dataset.reviewRevision + ']';
    thirdDecision.value = ''; thirdDecision.dispatchEvent(new Event('change'));
    Object.defineProperty(navigator, 'clipboard', {configurable:true, value:{writeText:async () => { throw new Error('Denied'); }}});
    document.querySelector('#copy-reviews').click();
    await new Promise(resolve => setTimeout(resolve, 0));
    const fallback = document.querySelector('#export-details').open && document.querySelector('#review-export').value === expected;
    select.value = ''; select.dispatchEvent(new Event('change'));
    note.value = ''; note.dispatchEvent(new Event('input'));
    document.querySelector('#export-details').open = false;
    return {exported, saved:saved.decision === 'approved', restoration, clipboard, noteOnly, decisionOnly, fallback,
      emptyDisabled:document.querySelector('#copy-reviews').disabled};
  })().then(value => window.feedbackCheck = value).catch(error => window.feedbackCheck = {error:String(error)})`);
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await evaluate(ws, ++id, '!!window.feedbackCheck')) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  result.feedback = await evaluate(ws, ++id, 'window.feedbackCheck || {}');
  result.status = result.cards === result.expectedCards && result.cards > 0 && result.namedCards &&
    result.animationStates === result.expectedAnimationStates && result.separateAnimationCards === 0 &&
    result.reportLinks && result.navigation === result.cards && result.solidToggle &&
    result.readyFilter && result.readyCards === result.expectedReadyCards && result.readyNavigation === result.readyCards &&
    result.firstImages.every(image => image.loaded) && result.failedImages.length === 0 &&
    !result.horizontalOverflow && ['exported', 'saved', 'restoration', 'clipboard', 'noteOnly', 'decisionOnly', 'fallback', 'emptyDisabled']
      .every(key => result.feedback[key] === true) ? 'PASS' : 'FAIL';
  await writeFile(join(output, 'gallery-browser.json'), JSON.stringify(result, null, 2) + '\n');
  const screenshot = await new Promise((resolve, reject) => {
    const request = ++id;
    const listener = event => {
      const data = JSON.parse(event.data);
      if (data.id !== request) return;
      ws.removeEventListener('message', listener);
      if (data.error) reject(new Error(JSON.stringify(data.error))); else resolve(data.result.data);
    };
    ws.addEventListener('message', listener);
    ws.send(JSON.stringify({id:request,method:'Page.captureScreenshot',params:{format:'png'}}));
  });
  await writeFile(join(output, 'gallery-browser.png'), Buffer.from(screenshot, 'base64'));
  console.log(JSON.stringify(result));
  if (result.status !== 'PASS') throw new Error('Review gallery browser check failed');
} catch (error) {
  await writeFile(join(output, 'gallery-browser-errors.txt'), browserErrors);
  console.error(browserErrors);
  throw error;
} finally {
  ws?.close();
  chrome.kill('SIGTERM');
  await closed;
}
