// One paced queue shared by every HTTP response, including worker requests.
// Payload-only shaping: no simulated RTT, packet loss, HTTP/2, or TCP overhead.
import { performance } from 'node:perf_hooks';
export class SharedBandwidth {
    constructor(bytesPerSecond, { now = () => performance.now(), schedule = setTimeout } = {}) {
        if (!(bytesPerSecond > 0) || !Number.isFinite(bytesPerSecond)) throw new Error('bandwidth must be finite and positive');
        this.rate = bytesPerSecond;
        this.now = now;
        this.schedule = schedule;
        this.queue = [];
        this.running = false;
    }
    send(response, body, onChunk = () => {}) {
        return new Promise((resolve, reject) => {
            this.queue.push({ response, body, onChunk, offset: 0, resolve, reject });
            if (!this.running) { this.running = true; this.deadline = this.now(); this.next(); }
        });
    }
    next() {
        const item = this.queue.shift();
        if (!item) { this.running = false; return; }
        if (item.response.destroyed) { item.resolve(); this.next(); return; }
        const size = Math.min(16384, item.body.length - item.offset);
        // Charge BEFORE writing; concurrent requests cannot each receive a burst.
        this.deadline += size / this.rate * 1000;
        const delay = Math.max(0, Math.ceil(this.deadline - this.now()));
        this.schedule(() => {
            try {
                if (item.response.destroyed) { item.resolve(); this.next(); return; }
                item.response.write(item.body.subarray(item.offset, item.offset + size));
                item.offset += size;
                item.onChunk(size, this.now());
                if (item.offset === item.body.length) { item.response.end(); item.resolve(); }
                else this.queue.push(item);
            } catch (error) { item.reject(error); }
            this.next();
        }, delay);
    }
}
