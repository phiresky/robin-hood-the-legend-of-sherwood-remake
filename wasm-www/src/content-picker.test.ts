import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { requestFullContentFolder } from './content-picker.ts';

test('content picker removes click/change listeners on selection and cancellation', async () => {
    for (const cancel of [false, true]) {
        const dom = new JSDOM('<section id="multiplayer-content-gate" hidden><input id="multiplayer-content-files" type="file"><button id="multiplayer-content-choose">Choose</button><p id="multiplayer-content-status"></p></section>');
        try {
            const document = dom.window.document;
            const controller = new AbortController();
            const input = document.querySelector('input')!;
            const choose = document.querySelector('button')!;
            let clicks = 0;
            input.click = () => { clicks++; };
            const pending = requestFullContentFolder(controller.signal, document);
            choose.click();
            assert.equal(clicks, 1);
            if (cancel) {
                controller.abort();
                await assert.rejects(pending, { name: 'AbortError' });
                assert.equal(document.querySelector('section')!.hidden, true);
            } else {
                input.dispatchEvent(new dom.window.Event('change'));
                assert.match(document.querySelector('p')!.textContent, /No folder selected/u);
                const files = { length: 1 };
                Object.defineProperty(input, 'files', { value: files });
                input.dispatchEvent(new dom.window.Event('change'));
                assert.equal(await pending, files);
            }
            const status = document.querySelector('p')!.textContent;
            choose.click();
            input.dispatchEvent(new dom.window.Event('change'));
            assert.equal(clicks, 1);
            assert.equal(document.querySelector('p')!.textContent, status);
        } finally { dom.window.close(); }
    }
});
