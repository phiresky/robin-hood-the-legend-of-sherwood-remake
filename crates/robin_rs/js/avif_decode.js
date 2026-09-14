// Browser-native AVIF decode for the wasm runtime.
//
// The web datadir ships every lossy image as AVIF (lossy colour, lossless
// alpha carrying the pixel class). Rust awaits this from the main thread's
// async install/boot paths and feeds the pixels to its synchronous decoders
// (robin_assets::browser_images). Worker threads never call it: a rayon
// worker never returns to its event loop, so a promise there cannot resolve.
//
// Exactness: createImageBitmap with premultiplyAlpha 'none' and
// colorSpaceConversion 'none', drawn into a 2D OffscreenCanvas and read with
// getImageData, returns exact alpha and exact colour for opaque pixels
// (verified against libaom's avifdec in Chrome 152 and Firefox 155). A 2D
// canvas stores premultiplied pixels, so colour of pixels with alpha below
// 255 may change — the runtime never reads colour there (alpha 0 is the
// transparent key, alpha 128 the shadow key).
//
// Concurrency: all createImageBitmap calls are issued before any is awaited,
// so browsers that decode images off the main thread (Chrome) decode the
// whole batch in parallel.

function decodeOne(bytes) {
    // Copy out of wasm memory: Blob rejects views of a SharedArrayBuffer
    // (threaded builds), and the view may be invalidated by memory growth.
    const blob = new Blob([bytes.slice()], { type: 'image/avif' });
    return createImageBitmap(blob, {
        premultiplyAlpha: 'none',
        colorSpaceConversion: 'none',
    });
}

function readPixels(bitmap) {
    const { width, height } = bitmap;
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext('2d', { willReadFrequently: true, colorSpace: 'srgb' });
    if (context === null) {
        bitmap.close();
        throw new Error('OffscreenCanvas 2D context unavailable for AVIF pixel readback');
    }
    context.drawImage(bitmap, 0, 0);
    bitmap.close();
    const image = context.getImageData(0, 0, width, height, { colorSpace: 'srgb' });
    return { width, height, rgba: new Uint8Array(image.data.buffer, image.data.byteOffset, image.data.byteLength) };
}

// blobs: Array<Uint8Array>. Resolves to Array<{width, height, rgba}> in the
// same order. Rejects with an Error naming the failing index.
export async function robinhoodDecodeAvifBatch(blobs) {
    const pending = blobs.map((bytes, index) =>
        decodeOne(bytes).catch((error) => {
            throw new Error(`AVIF image ${index} (${bytes.length} bytes) failed to decode: ${error}`);
        }),
    );
    const bitmaps = await Promise.allSettled(pending);
    const failure = bitmaps.find((result) => result.status === 'rejected');
    if (failure !== undefined) {
        for (const result of bitmaps) {
            if (result.status === 'fulfilled') {
                result.value.close();
            }
        }
        throw failure.reason;
    }
    return bitmaps.map((result) => readPixels(result.value));
}
