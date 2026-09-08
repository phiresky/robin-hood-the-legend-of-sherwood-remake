import type { HighscoreApi } from './api.js';
import type { LeaderboardSigningBridge } from './signing.js';
import type { PlayerProfile, DeletionTarget, AbuseReportCategory } from './types.js';
import { element } from './dom.js';
import { formatDate } from './format.js';
import { updateUsername, deleteRecord, reportRecord, usernameValidationError } from './account-actions.js';

export function renderUsernameForm(
    api: HighscoreApi,
    profile: PlayerProfile,
    bridge: LeaderboardSigningBridge,
    signal: AbortSignal,
): HTMLElement {
    const section = element('section', { className: 'owner-action' });
    section.append(element('h2', { text: 'Change display name' }));
    const form = element('form', { className: 'moderation-form' });
    const label = element('label', { text: 'Display name' });
    const input = element('input', {
        attrs: {
            type: 'text', required: '', maxlength: '48', autocomplete: 'nickname',
            value: profile.username, 'aria-describedby': 'username-owner-help',
        },
    });
    label.append(input);
    const help = element('p', {
        className: 'fingerprint',
        text: '1–48 UTF-8 bytes. Names are public, mutable, and not unique; your fingerprint remains the durable identity.',
        attrs: { id: 'username-owner-help' },
    });
    const submit = element('button', {
        className: 'button secondary', text: 'Sign and update name', attrs: { type: 'submit' },
    });
    const status = element('p', { className: 'fingerprint', attrs: { role: 'status' } });
    form.append(label, help, submit, status);
    form.addEventListener('submit', event => {
        event.preventDefault();
        const username = input.value;
        const error = usernameValidationError(username);
        if (error !== null) {
            input.setCustomValidity(error);
            input.reportValidity();
            return;
        }
        input.setCustomValidity('');
        submit.disabled = true;
        status.textContent = 'Requesting a one-use rename challenge…';
        void (async () => {
            const updated = await updateUsername(api, bridge, profile, username, signal,
                message => { status.textContent = message; });
            input.value = updated.username;
            status.textContent = `Display name updated to ${updated.username}. Fingerprint ${updated.publicKeyFingerprint} is unchanged.`;
        })().catch(errorValue => {
            if (signal.aborted) return;
            status.textContent = errorValue instanceof Error ? errorValue.message : String(errorValue);
        }).finally(() => {
            if (signal.aborted) return;
            submit.disabled = false;
        });
    });
    section.append(form);
    return section;
}

export function renderDeletionForm(
    api: HighscoreApi,
    bridge: LeaderboardSigningBridge,
    target: DeletionTarget,
    signal: AbortSignal,
): HTMLElement {
    const details = element('details');
    details.append(element('summary', { text: 'Delete my public record' }));
    const form = element('form', { className: 'moderation-form danger-zone' });
    const consequence = target.kind === 'run'
        ? 'This immediately removes the run from rankings and tombstones its replay. Physical deletion may occur later and shared replay objects remain while referenced.'
        : 'This immediately tombstones the submission and removes any associated public ranking and replay. Physical deletion may occur later.';
    const confirmLabel = element('label', { className: 'confirm-action' });
    const confirm = element('input', { attrs: { type: 'checkbox', required: '' } });
    confirmLabel.append(confirm, document.createTextNode(` I understand: ${consequence}`));
    const submit = element('button', {
        className: 'button danger', text: 'Sign deletion request', attrs: { type: 'submit' },
    });
    submit.disabled = true;
    confirm.addEventListener('change', () => { submit.disabled = !confirm.checked; });
    const status = element('p', { className: 'fingerprint', attrs: { role: 'status' } });
    form.append(element('p', { text: consequence }), confirmLabel, submit, status);
    form.addEventListener('submit', event => {
        event.preventDefault();
        if (!confirm.checked) {
            confirm.reportValidity();
            return;
        }
        submit.disabled = true;
        confirm.disabled = true;
        status.textContent = 'Requesting a one-use deletion challenge…';
        void (async () => {
            const receipt = await deleteRecord(api, bridge, target, signal,
                message => { status.textContent = message; });
            const retention = receipt.purgeEligibleAtUnixMs === null
                ? 'No automatic physical purge date is configured.'
                : `Physical purge is eligible after ${formatDate(receipt.purgeEligibleAtUnixMs)}.`;
            status.textContent = `Record tombstoned at ${formatDate(receipt.tombstonedAtUnixMs)}. ${retention}`;
        })().catch(errorValue => {
            if (signal.aborted) return;
            status.textContent = errorValue instanceof Error ? errorValue.message : String(errorValue);
            confirm.disabled = false;
            submit.disabled = !confirm.checked;
        });
    });
    details.append(form);
    return details;
}

export function renderReportForm(
    api: HighscoreApi,
    target: { readonly kind: 'run'; readonly run_id: string }
        | { readonly kind: 'player'; readonly public_key: string },
    signal: AbortSignal,
): HTMLElement {
    const details = element('details');
    details.append(element('summary', { text: 'Report this public record' }));
    const form = element('form', { className: 'moderation-form' });
    const categoryLabel = element('label', { text: 'Category' });
    const category = element('select', { attrs: { required: '', 'aria-label': 'Report category' } });
    const categories: readonly (readonly [AbuseReportCategory, string])[] = [
        ['suspected_cheating', 'Suspected cheating'],
        ['offensive_identity', 'Offensive identity'],
        ['privacy', 'Privacy'],
        ['copyright', 'Copyright'],
        ['other', 'Other'],
    ];
    for (const [value, label] of categories) {
        category.append(element('option', { text: label, attrs: { value } }));
    }
    categoryLabel.append(category);
    const detailLabel = element('label', { text: 'Details' });
    const detail = element('textarea', {
        attrs: {
            required: '', maxlength: '2000', rows: '5',
            placeholder: 'Give moderators enough specific, non-sensitive detail to review this report.',
        },
    });
    detailLabel.append(detail);
    const submit = element('button', {
        className: 'button secondary', text: 'Submit report', attrs: { type: 'submit' },
    });
    const status = element('p', { className: 'fingerprint', attrs: { role: 'status' } });
    form.append(categoryLabel, detailLabel, submit, status);
    form.addEventListener('submit', event => {
        event.preventDefault();
        const reportDetail = detail.value.trim();
        if (reportDetail.length === 0 || new TextEncoder().encode(reportDetail).byteLength > 2_000) {
            detail.setCustomValidity('Enter 1–2000 bytes without surrounding whitespace.');
            detail.reportValidity();
            return;
        }
        detail.setCustomValidity('');
        submit.disabled = true;
        status.textContent = 'Submitting report…';
        void reportRecord(api,
            target,
            category.value as AbuseReportCategory,
            reportDetail,
            signal,
        ).then(receipt => {
            signal.throwIfAborted();
            status.textContent = `Report ${receipt.reportId} was queued at ${formatDate(receipt.receivedAtUnixMs)}. This acknowledgement does not hide or change the record.`;
            form.reset();
        }).catch(error => {
            if (signal.aborted) return;
            status.textContent = error instanceof Error ? error.message : String(error);
        }).finally(() => {
            if (signal.aborted) return;
            submit.disabled = false;
        });
    });
    details.append(form);
    return details;
}
