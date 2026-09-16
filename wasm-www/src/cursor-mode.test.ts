import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { installCursorMode } from './cursor-mode.ts';

test('capture mode follows successful lock, Escape, and refusal; default is free', async () => {
    const dom = new JSDOM('<canvas></canvas><button></button><span></span>');
    const document = dom.window.document;
    const canvas = document.querySelector('canvas')!;
    const button = document.querySelector('button')!;
    const status = document.querySelector('span')!;
    let locked: Element | null = null;
    let fail = false;
    Object.defineProperty(document, 'pointerLockElement', { get: () => locked });
    canvas.requestPointerLock = async () => {
        if (fail) throw new Error('Denied');
        locked = canvas;
        document.dispatchEvent(new dom.window.Event('pointerlockchange'));
    };
    document.exitPointerLock = () => {
        locked = null;
        document.dispatchEvent(new dom.window.Event('pointerlockchange'));
    };
    const dispose = installCursorMode(canvas, button, status);
    assert.match(status.textContent!, /off/);
    button.click();
    assert.equal(button.getAttribute('aria-pressed'), 'true');
    document.exitPointerLock();
    assert.equal(button.getAttribute('aria-pressed'), 'false');
    assert.match(status.textContent!, /off/);
    fail = true;
    button.click();
    await Promise.resolve();
    assert.match(status.textContent!, /failed/);
    assert.equal(locked, null);
    dispose();
    dom.window.close();
});
