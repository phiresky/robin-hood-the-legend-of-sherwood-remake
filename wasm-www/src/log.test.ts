import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { appendLogLine, appendLogLines } from './log.ts';

test('console batches preserve styled lines with one scroll measurement', () => {
    const dom = new JSDOM('<div id="log"></div>');
    const target = dom.window.document.querySelector<HTMLElement>('#log')!;
    let measurements = 0;
    Object.defineProperty(target, 'scrollHeight', { get: () => { measurements++; return 123; } });
    appendLogLines(target, [
        { text: 'first' },
        { text: '\x1b[31mred\x1b[0m plain', cls: 'err' },
        { text: 'last' },
    ]);
    assert.deepEqual([...target.children].map(line => line.textContent), ['first', 'red plain', 'last']);
    assert.equal(target.children[1]!.className, 'err');
    assert.equal(target.querySelector('span')!.style.color, 'rgb(224, 108, 117)');
    assert.equal(target.scrollTop, 123);
    assert.equal(measurements, 1);
    appendLogLines(target, []);
    assert.equal(measurements, 1);
    dom.window.close();
});

test('large and successive console batches retain exactly the newest 600 lines', () => {
    const dom = new JSDOM('<div id="log"></div>');
    const target = dom.window.document.querySelector<HTMLElement>('#log')!;
    appendLogLine(target, 'old');
    appendLogLines(target, Array.from({ length: 1000 }, (_, i) => ({ text: String(i) })));
    assert.equal(target.childElementCount, 600);
    assert.equal(target.firstElementChild!.textContent, '400');
    assert.equal(target.lastElementChild!.textContent, '999');
    appendLogLine(target, 'new');
    assert.equal(target.childElementCount, 600);
    assert.equal(target.firstElementChild!.textContent, '401');
    assert.equal(target.lastElementChild!.textContent, 'new');
    dom.window.close();
});
