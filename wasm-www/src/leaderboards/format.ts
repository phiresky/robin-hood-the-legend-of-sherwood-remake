import {
    type BoardMetricValue,
} from './types.js';

export function formatDuration(milliseconds: number): string {
    const totalTenths = Math.max(0, Math.floor(milliseconds / 100));
    const hours = Math.floor(totalTenths / 36_000);
    const minutes = Math.floor((totalTenths % 36_000) / 600);
    const seconds = Math.floor((totalTenths % 600) / 10);
    const tenths = totalTenths % 10;
    const prefix = hours > 0 ? `${hours}:${pad2(minutes)}:` : `${minutes}:`;
    return `${prefix}${pad2(seconds)}.${tenths}`;
}

export function formatInteger(value: number): string {
    return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(value);
}

export function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

export function formatDate(timestamp: string | number): string {
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })
        .format(new Date(timestamp));
}

export function formatMetricValue(value: BoardMetricValue): string {
    switch (value.metric) {
        case 'original_score':
            return formatInteger(value.points);
        case 'fastest_success': {
            const microseconds = value.activeSimulationTicks * value.tickDurationMicros;
            if (!Number.isSafeInteger(microseconds)) {
                return 'Duration too large';
            }
            return formatDuration(microseconds / 1000);
        }
    }
}

function pad2(value: number): string {
    return String(value).padStart(2, '0');
}
