import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { installCanvasBackingStore } from './canvas-lifecycle.ts';

test('canvas lifecycle preserves HiDPI/aspect/fullscreen sizing and ignores callbacks after disposal', t => {
    const dom = new JSDOM('<canvas></canvas>');
    t.after(() => dom.window.close());
    const canvas = dom.window.document.querySelector('canvas')!;
    canvas.getBoundingClientRect = () => ({ width: 800, height: 600 }) as DOMRect;
    let viewport = { innerWidth: 1200, innerHeight: 900, fullscreen: false, devicePixelRatio: 2 };
    let subscribed!: () => void, disposals = 0;
    const lifecycle = installCanvasBackingStore(canvas, {
        viewport: () => viewport,
        subscribe: sync => { subscribed = sync; return () => { disposals++; }; },
    });
    assert.equal(canvas.width, 1600); assert.equal(canvas.height, 1200);
    assert.equal(canvas.style.width, '1184px'); assert.equal(canvas.style.height, '884px');
    viewport = { innerWidth: 1600, innerHeight: 900, fullscreen: true, devicePixelRatio: 1 };
    subscribed();
    assert.equal(canvas.style.width, '1600px'); assert.equal(canvas.style.height, '900px');
    assert.equal(canvas.width, 800);
    lifecycle.dispose(); lifecycle.dispose();
    assert.equal(disposals, 1);
    viewport = { ...viewport, devicePixelRatio: 3 };
    subscribed(); lifecycle.sync();
    assert.equal(canvas.width, 800);
});
