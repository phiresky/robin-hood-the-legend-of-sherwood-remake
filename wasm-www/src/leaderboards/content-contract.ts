// Official mission/campaign content contracts.
import {
    type ContentManifest,
    type CampaignContentManifest,
    type FullCampaignSession,
    type OfficialContentSubject,
} from './types.js';
import {
    versionedObject,
    array,
    strictObject,
    enumeration,
    u32Positive,
    boundedString,
    resourceLocaleRoot,
    nonzeroSha256,
    object,
    assertExactKeys,
} from './decode.js';
import { parseArtifactRef } from './build-contract.js';
import { verifyCanonicalDocument, compareUtf8, type DigestVerified } from './canonical.js';

export function parseContentManifest(value: unknown): ContentManifest {
    const obj = versionedObject(value, 'content_manifest', [
        'name', 'edition', 'subject', 'closure', 'projection_schema_version', 'resource_locale_root',
        'speech_timing', 'components',
    ]);
    const requiredKinds = [
        'profiles', 'loaded_level', 'mission_scripts', 'sprite_simulation_metadata',
        'map_geometry_metadata', 'localized_deterministic_text', 'sound_duration_tables',
        'interface_simulation_metadata',
    ] as const;
    const components = array(obj.components, 'content_manifest.components').map((item, index) => {
        const path = `content_manifest.components[${index}]`;
        const component = strictObject(item, path, ['kind', 'component_schema_version', 'artifact']);
        const artifact = parseArtifactRef(component.artifact, `${path}.artifact`);
        if (artifact.mediaType !== 'application/vnd.robinhood.simulation-content-component-v2+bitcode') {
            throw new Error(`${path}.artifact has the wrong simulation-component media type`);
        }
        return {
            kind: enumeration(component.kind, requiredKinds, `${path}.kind`),
            componentSchemaVersion: u32Positive(
                component.component_schema_version,
                `${path}.component_schema_version`,
            ),
            artifact,
        };
    });
    if (components.length !== requiredKinds.length
        || components.some((component, index) => component.kind !== requiredKinds[index])) {
        throw new Error('content_manifest.components must be the complete canonical simulation projection');
    }
    return {
        name: boundedString(obj.name, 'content_manifest.name', 256),
        edition: enumeration(obj.edition, ['demo', 'full'] as const, 'content_manifest.edition'),
        subject: parseOfficialContentSubject(obj.subject, 'content_manifest.subject'),
        closure: enumeration(
            obj.closure,
            ['static_prepared_mission_content_projection'] as const,
            'content_manifest.closure',
        ),
        projectionSchemaVersion: u32Positive(
            obj.projection_schema_version,
            'content_manifest.projection_schema_version',
        ),
        resourceLocaleRoot: resourceLocaleRoot(
            obj.resource_locale_root,
            'content_manifest.resource_locale_root',
        ),
        speechTiming: parseSimulationSpeechTiming(obj.speech_timing, 'content_manifest.speech_timing'),
        components,
    };
}

export function parseCampaignContentManifest(value: unknown): CampaignContentManifest {
    const obj = versionedObject(value, 'campaign_content_manifest', ['edition', 'entries']);
    const entries = array(obj.entries, 'campaign_content_manifest.entries').map((item, index) => {
        const path = `campaign_content_manifest.entries[${index}]`;
        const entry = strictObject(item, path, ['subject', 'content_manifest_sha256']);
        return {
            subject: parseOfficialContentSubject(entry.subject, `${path}.subject`),
            contentManifestSha256: nonzeroSha256(
                entry.content_manifest_sha256,
                `${path}.content_manifest_sha256`,
            ),
        };
    });
    if (entries.length === 0 || entries.length > 4096
        || entries.slice(1).some((entry, index) => compareContentSubjects(entries[index]!.subject, entry.subject) >= 0)) {
        throw new Error('campaign_content_manifest.entries must be non-empty and canonically ordered');
    }
    return {
        edition: enumeration(obj.edition, ['demo', 'full'] as const, 'campaign_content_manifest.edition'),
        entries,
    };
}

export async function parseAndVerifyContentManifest(value: unknown, expectedSha256: string): Promise<DigestVerified<ContentManifest>> {
    const parsed = parseContentManifest(value);
    return await verifyCanonicalDocument(value, parsed, expectedSha256, 'content_manifest');
}

export async function parseAndVerifyCampaignContentManifest(
    value: unknown,
    expectedSha256: string,
): Promise<DigestVerified<CampaignContentManifest>> {
    const parsed = parseCampaignContentManifest(value);
    return await verifyCanonicalDocument(value, parsed, expectedSha256, 'campaign_content_manifest');
}

export function parseOfficialContentSubject(
    value: unknown,
    path: string,
): FullCampaignSession['contentSubject'] {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['field_mission', 'headquarters'] as const, `${path}.kind`);
    assertExactKeys(obj, path, ['kind', 'mission_id']);
    return {
        kind,
        missionId: boundedString(obj.mission_id, `${path}.mission_id`, 256),
    };
}

export function parseSimulationSpeechTiming(
    value: unknown,
    path: string,
): ContentManifest['speechTiming'] {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['base_installation', 'language_pack'] as const, `${path}.kind`);
    if (kind === 'base_installation') {
        assertExactKeys(obj, path, ['kind']);
        return { kind };
    }
    assertExactKeys(obj, path, ['kind', 'canonical_locale']);
    const canonicalLocale = boundedString(obj.canonical_locale, `${path}.canonical_locale`, 64);
    if (canonicalLocale.startsWith('-') || canonicalLocale.endsWith('-')
        || canonicalLocale.split('-').some(part => part.length === 0 || part.length > 8)
        || !/^[A-Za-z0-9-]+$/u.test(canonicalLocale)) {
        throw new Error(`${path}.canonical_locale is not canonical`);
    }
    return { kind, canonicalLocale };
}

export function compareContentSubjects(left: OfficialContentSubject, right: OfficialContentSubject): number {
    if (left.kind !== right.kind) return left.kind === 'field_mission' ? -1 : 1;
    return compareUtf8(left.missionId, right.missionId);
}
