// Subject and content identity decoding.
import { type MissionFacet, type LeaderboardSubject, type RunContentIdentity } from './types.js';
import {
    strictObject,
    boundedString,
    nonzeroSha256,
    object,
    enumeration,
    assertExactKeys,
    CATEGORY_ORDER,
} from './decode.js';

export function parseMission(value: unknown, path: string): MissionFacet {
    const obj = strictObject(value, path, ['mission_id', 'display_name', 'content_manifest_sha256']);
    return {
        id: boundedString(obj.mission_id, `${path}.mission_id`, 256),
        label: boundedString(obj.display_name, `${path}.display_name`, 100),
        contentManifestSha256: nonzeroSha256(obj.content_manifest_sha256, `${path}.content_manifest_sha256`),
    };
}

export function parseSubject(value: unknown, path: string): LeaderboardSubject {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['mission', 'full_campaign'] as const, `${path}.kind`);
    if (kind === 'full_campaign') {
        assertExactKeys(obj, path, ['kind']);
        return { kind };
    }
    assertExactKeys(obj, path, ['kind', 'mission_id', 'category']);
    return {
        kind,
        missionId: boundedString(obj.mission_id, `${path}.mission_id`, 256),
        category: enumeration(obj.category, CATEGORY_ORDER, `${path}.category`),
    };
}

export function parseRunContentIdentity(
    value: unknown,
    path: string,
    subject?: LeaderboardSubject,
): RunContentIdentity {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['mission', 'full_campaign'] as const, `${path}.kind`);
    const parsed: RunContentIdentity = kind === 'mission'
        ? (assertExactKeys(obj, path, ['kind', 'content_manifest_sha256']), {
            kind,
            contentManifestSha256: nonzeroSha256(
                obj.content_manifest_sha256,
                `${path}.content_manifest_sha256`,
            ),
        })
        : (assertExactKeys(obj, path, ['kind', 'campaign_content_manifest_sha256']), {
            kind,
            campaignContentManifestSha256: nonzeroSha256(
                obj.campaign_content_manifest_sha256,
                `${path}.campaign_content_manifest_sha256`,
            ),
        });
    if (subject !== undefined && parsed.kind !== subject.kind) {
        throw new Error(`${path} does not match its leaderboard subject`);
    }
    return parsed;
}

export function runContentDigest(content: RunContentIdentity): string {
    return content.kind === 'mission'
        ? content.contentManifestSha256
        : content.campaignContentManifestSha256;
}
