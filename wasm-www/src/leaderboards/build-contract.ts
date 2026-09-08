// Versioned build authority, artifact closure and replay contracts.
import runtimeContract from '../../runtime-contract.json' with { type: 'json' };
import {
    type BuildManifest,
    type BuildManifestViewerEngine,
    type HistoricalBuildManifestV1,
    type PublicBuildManifestV2,
    type VerifierBuildIdentityV2,
    type BrowserViewerEngineBuildIdentityV2,
    type BrowserPagesShellBuildIdentityV2,
    type BrowserIdentitySignerBuildIdentityV2,
    type RustToolchainAuthority,
    type BuildToolAuthority,
    type NamedArtifact,
    type BrowserPagesArtifact,
    type ReplayArtifact,
    type ArtifactRef,
    type ViewerArtifactRole,
} from './types.js';
import {
    object,
    versionedObject,
    array,
    boundedString,
    strictlySorted,
    gitCommit,
    nonzeroSha256,
    u32Positive,
    strictObject,
    exactStringArray,
    exactString,
    viewerArtifactPath,
    positiveInteger,
    enumeration,
    assertExactKeys,
} from './decode.js';
import { canonicalDocumentSha256Sync, verifyCanonicalDocument, type DigestVerified } from './canonical.js';

export const CURRENT_RANKED_REPLAY_SCHEMA_VERSION = runtimeContract.replaySchema;

export const WASM_BINDGEN_CLI_VERSION = '0.2.127' as const;

export const WASM_BINDGEN_CLI_AUTHORITY_SHA256 =
    '68ee22d8da662e20a7aa63d43354c59194530378898e2052079b9084580b89ba' as const;

/**
 * Selects only the authenticated engine closure that replay playback may
 * fetch. V2 static-shell and identity-signer inventories are metadata for
 * deployment verification and intentionally never enter this view.
 */
export function buildManifestViewerEngine(manifest: BuildManifest): BuildManifestViewerEngine {
    return manifest.schemaVersion === 2
        ? manifest.viewer.engine
        : {
            targetTriple: manifest.targetTriple,
            cargoProfile: manifest.cargoProfile,
            cargoFeatures: manifest.cargoFeatures,
            artifacts: manifest.viewerArtifacts,
        };
}

export function parseBuildManifest(value: unknown): BuildManifest {
    const root = object(value, 'build_manifest');
    if (root.schema_version === 1) return parseHistoricalBuildManifestV1(value);
    if (root.schema_version === 2) return parsePublicBuildManifestV2(value);
    throw new Error('build_manifest.schema_version must be 1 or 2');
}

export function parseHistoricalBuildManifestV1(value: unknown): HistoricalBuildManifestV1 {
    const obj = versionedObject(value, 'build_manifest', [
        'source_commit', 'cargo_lock_sha256', 'target_triple', 'cargo_profile', 'cargo_features',
        'replay_schema_version', 'save_schema_version', 'network_protocol_version', 'verifier',
        'viewer_artifacts',
    ]);
    const cargoFeatures = array(obj.cargo_features, 'build_manifest.cargo_features').map((feature, index) =>
        boundedString(feature, `build_manifest.cargo_features[${index}]`, 128));
    if (!strictlySorted(cargoFeatures)) {
        throw new Error('build_manifest.cargo_features must be strictly sorted without duplicates');
    }
    const viewerArtifacts = parseViewerArtifactClosure(
        obj.viewer_artifacts,
        'build_manifest.viewer_artifacts',
    );
    return {
        schemaVersion: 1,
        sourceCommit: gitCommit(obj.source_commit, 'build_manifest.source_commit'),
        cargoLockSha256: nonzeroSha256(obj.cargo_lock_sha256, 'build_manifest.cargo_lock_sha256'),
        targetTriple: boundedString(obj.target_triple, 'build_manifest.target_triple', 128),
        cargoProfile: boundedString(obj.cargo_profile, 'build_manifest.cargo_profile', 64),
        cargoFeatures,
        replaySchemaVersion: u32Positive(obj.replay_schema_version, 'build_manifest.replay_schema_version'),
        saveSchemaVersion: u32Positive(obj.save_schema_version, 'build_manifest.save_schema_version'),
        networkProtocolVersion: u32Positive(
            obj.network_protocol_version,
            'build_manifest.network_protocol_version',
        ),
        verifier: parseArtifactRef(obj.verifier, 'build_manifest.verifier'),
        viewerArtifacts,
    };
}

export function parsePublicBuildManifestV2(value: unknown): PublicBuildManifestV2 {
    const obj = strictObject(value, 'build_manifest', [
        'schema_version', 'source_commit', 'cargo_lock_sha256', 'replay_schema_version',
        'save_schema_version', 'network_protocol_version', 'verifier', 'viewer',
    ]);
    if (obj.schema_version !== 2) throw new Error('build_manifest.schema_version must be 2');
    const viewerRaw = strictObject(obj.viewer, 'build_manifest.viewer', [
        'engine', 'pages_shell', 'identity_signer',
    ]);
    const engine = parseBrowserViewerEngineV2(viewerRaw.engine);
    const pagesShell = parseBrowserPagesShellV2(viewerRaw.pages_shell);
    const identitySigner = parseBrowserIdentitySignerV2(viewerRaw.identity_signer);
    const publicDigests = new Set([
        ...engine.artifacts.map(item => item.artifact.sha256),
        ...pagesShell.publicOriginArtifacts.map(item => item.artifact.sha256),
    ]);
    if (identitySigner.identitySignerOriginArtifacts.some(item => publicDigests.has(item.artifact.sha256))) {
        throw new Error('build_manifest.viewer contains a cross-origin artifact substitution');
    }
    return {
        schemaVersion: 2,
        sourceCommit: gitCommit(obj.source_commit, 'build_manifest.source_commit'),
        cargoLockSha256: nonzeroSha256(obj.cargo_lock_sha256, 'build_manifest.cargo_lock_sha256'),
        replaySchemaVersion: u32Positive(obj.replay_schema_version, 'build_manifest.replay_schema_version'),
        saveSchemaVersion: u32Positive(obj.save_schema_version, 'build_manifest.save_schema_version'),
        networkProtocolVersion: u32Positive(
            obj.network_protocol_version,
            'build_manifest.network_protocol_version',
        ),
        verifier: parseVerifierBuildIdentityV2(obj.verifier),
        viewer: { engine, pagesShell, identitySigner },
    };
}

export function parseVerifierBuildIdentityV2(value: unknown): VerifierBuildIdentityV2 {
    const path = 'build_manifest.verifier';
    const obj = strictObject(value, path, [
        'platform', 'target_triple', 'cargo_profile', 'cargo_features', 'cargo_package',
        'cargo_binary', 'linkage', 'artifact',
    ]);
    const cargoFeatures = exactStringArray(obj.cargo_features, `${path}.cargo_features`, []);
    const artifact = parseArtifactRef(obj.artifact, `${path}.artifact`);
    if (artifact.mediaType !== 'application/vnd.robinhood.ranked-replay-verifier-v2') {
        throw new Error(`${path}.artifact has the wrong ranked-verifier media type`);
    }
    return {
        platform: exactString(obj.platform, 'x86_64_unknown_linux_musl', `${path}.platform`),
        targetTriple: exactString(obj.target_triple, 'x86_64-unknown-linux-musl', `${path}.target_triple`),
        cargoProfile: exactString(obj.cargo_profile, 'release', `${path}.cargo_profile`),
        cargoFeatures,
        cargoPackage: exactString(obj.cargo_package, 'robin_replay_verifier', `${path}.cargo_package`),
        cargoBinary: exactString(obj.cargo_binary, 'robin-replay-verifier', `${path}.cargo_binary`),
        linkage: exactString(
            obj.linkage,
            'fully_static_no_interpreter_or_needed_libraries',
            `${path}.linkage`,
        ),
        artifact,
    };
}

export function parseBrowserViewerEngineV2(value: unknown): BrowserViewerEngineBuildIdentityV2 {
    const path = 'build_manifest.viewer.engine';
    const obj = strictObject(value, path, [
        'target_triple', 'cargo_profile', 'cargo_features', 'cargo_package', 'cargo_binary',
        'recipe', 'rust_toolchain', 'rust_toolchain_sha256', 'wasm_bindgen_cli',
        'binaryen_wasm_opt', 'wabt_wasm_strip', 'artifacts',
    ]);
    const artifacts = parseViewerArtifactClosure(obj.artifacts, `${path}.artifacts`);
    const entry = artifacts.find(item => item.role.kind === 'entry_java_script');
    const wasm = artifacts.find(item => item.role.kind === 'web_assembly');
    if (entry?.path !== 'viewer/robin.js' || wasm?.path !== 'viewer/robin_bg.wasm') {
        throw new Error(`${path}.artifacts does not contain the canonical published engine bundle`);
    }
    const rustToolchain = parseRustToolchainAuthority(obj.rust_toolchain, `${path}.rust_toolchain`);
    const rustToolchainSha256 = nonzeroSha256(
        obj.rust_toolchain_sha256,
        `${path}.rust_toolchain_sha256`,
    );
    if (canonicalDocumentSha256Sync(obj.rust_toolchain) !== rustToolchainSha256) {
        throw new Error(`${path}.rust_toolchain does not match rust_toolchain_sha256`);
    }
    const wasmBindgenCli = parseBuildToolAuthority(obj.wasm_bindgen_cli, `${path}.wasm_bindgen_cli`);
    if (wasmBindgenCli.version !== WASM_BINDGEN_CLI_VERSION
        || wasmBindgenCli.authoritySha256 !== WASM_BINDGEN_CLI_AUTHORITY_SHA256) {
        throw new Error(`${path}.wasm_bindgen_cli must use the exact accepted 0.2.127 authority`);
    }
    return {
        targetTriple: exactString(obj.target_triple, 'wasm32-unknown-unknown', `${path}.target_triple`),
        cargoProfile: exactString(obj.cargo_profile, 'wasm-release', `${path}.cargo_profile`),
        cargoFeatures: exactStringArray(obj.cargo_features, `${path}.cargo_features`, ['audio']),
        cargoPackage: exactString(obj.cargo_package, 'robin_rs', `${path}.cargo_package`),
        cargoBinary: exactString(obj.cargo_binary, 'robin', `${path}.cargo_binary`),
        recipe: exactString(
            obj.recipe,
            'wasm_bindgen_web_binaryen_oz_strip_debug_dwarf_wabt_strip_v1',
            `${path}.recipe`,
        ),
        rustToolchain,
        rustToolchainSha256,
        wasmBindgenCli,
        binaryenWasmOpt: parseBuildToolAuthority(obj.binaryen_wasm_opt, `${path}.binaryen_wasm_opt`),
        wabtWasmStrip: parseBuildToolAuthority(obj.wabt_wasm_strip, `${path}.wabt_wasm_strip`),
        artifacts,
    };
}

export function parseBrowserPagesShellV2(value: unknown): BrowserPagesShellBuildIdentityV2 {
    const path = 'build_manifest.viewer.pages_shell';
    const obj = strictObject(value, path, [
        'recipe', 'node', 'pnpm', 'package_json_sha256', 'pnpm_lock_sha256',
        'public_origin_artifacts',
    ]);
    const node = parseBuildToolAuthority(obj.node, `${path}.node`);
    const pnpm = parseBuildToolAuthority(obj.pnpm, `${path}.pnpm`);
    if (node.version !== '24.19.0' || pnpm.version !== '9.15.0') {
        throw new Error(`${path} must use exact Node 24.19.0 and pnpm 9.15.0 authorities`);
    }
    const publicOriginArtifacts = parseBrowserOriginClosure(
        obj.public_origin_artifacts,
        `${path}.public_origin_artifacts`,
    );
    if (!publicOriginArtifacts.some(item =>
        item.path === 'index.html' && item.artifact.mediaType === 'text/html')) {
        throw new Error(`${path}.public_origin_artifacts is missing index.html`);
    }
    return {
        recipe: exactString(
            obj.recipe,
            'pnpm_frozen_lockfile_vite_static_shell_v1',
            `${path}.recipe`,
        ),
        node,
        pnpm,
        packageJsonSha256: nonzeroSha256(obj.package_json_sha256, `${path}.package_json_sha256`),
        pnpmLockSha256: nonzeroSha256(obj.pnpm_lock_sha256, `${path}.pnpm_lock_sha256`),
        publicOriginArtifacts,
    };
}

export function parseBrowserIdentitySignerV2(value: unknown): BrowserIdentitySignerBuildIdentityV2 {
    const path = 'build_manifest.viewer.identity_signer';
    const obj = strictObject(value, path, [
        'target_triple', 'cargo_profile', 'cargo_features', 'cargo_package', 'cargo_binary',
        'recipe', 'deployment_policy', 'rust_toolchain', 'rust_toolchain_sha256',
        'wasm_bindgen_cli', 'identity_signer_origin_artifacts',
    ]);
    const rustToolchain = parseRustToolchainAuthority(obj.rust_toolchain, `${path}.rust_toolchain`);
    const rustToolchainSha256 = nonzeroSha256(
        obj.rust_toolchain_sha256,
        `${path}.rust_toolchain_sha256`,
    );
    if (canonicalDocumentSha256Sync(obj.rust_toolchain) !== rustToolchainSha256) {
        throw new Error(`${path}.rust_toolchain does not match rust_toolchain_sha256`);
    }
    const wasmBindgenCli = parseBuildToolAuthority(obj.wasm_bindgen_cli, `${path}.wasm_bindgen_cli`);
    if (wasmBindgenCli.version !== WASM_BINDGEN_CLI_VERSION
        || wasmBindgenCli.authoritySha256 !== WASM_BINDGEN_CLI_AUTHORITY_SHA256) {
        throw new Error(`${path}.wasm_bindgen_cli must use the exact accepted 0.2.127 authority`);
    }
    const identitySignerOriginArtifacts = parseBrowserOriginClosure(
        obj.identity_signer_origin_artifacts,
        `${path}.identity_signer_origin_artifacts`,
    );
    for (const [requiredPath, mediaType] of [
        ['identity-signer/index.html', 'text/html'],
        ['identity-signer/bridge/leaderboard_identity_bridge.js', 'text/javascript'],
        ['identity-signer/bridge/leaderboard_identity_bridge_bg.wasm', 'application/wasm'],
    ] as const) {
        if (!identitySignerOriginArtifacts.some(item =>
            item.path === requiredPath && item.artifact.mediaType === mediaType)) {
            throw new Error(`${path}.identity_signer_origin_artifacts is missing ${requiredPath}`);
        }
    }
    return {
        targetTriple: exactString(obj.target_triple, 'wasm32-unknown-unknown', `${path}.target_triple`),
        cargoProfile: exactString(obj.cargo_profile, 'wasm-release', `${path}.cargo_profile`),
        cargoFeatures: exactStringArray(
            obj.cargo_features,
            `${path}.cargo_features`,
            ['identity-signer-bridge'],
        ),
        // Keep the old package identity for historical signed manifests only.
        cargoPackage: enumeration(obj.cargo_package, ['robin_rs', 'robin_identity_signer'], `${path}.cargo_package`),
        cargoBinary: exactString(
            obj.cargo_binary,
            'leaderboard_identity_bridge',
            `${path}.cargo_binary`,
        ),
        recipe: exactString(
            obj.recipe,
            'wasm_bindgen_web_separate_origin_bridge_v1',
            `${path}.recipe`,
        ),
        deploymentPolicy: exactString(
            obj.deployment_policy,
            'separate_allowlisted_origin_csp_frame_ancestors_and_bridge_sha_v1',
            `${path}.deployment_policy`,
        ),
        rustToolchain,
        rustToolchainSha256,
        wasmBindgenCli,
        identitySignerOriginArtifacts,
    };
}

export function parseRustToolchainAuthority(value: unknown, path: string): RustToolchainAuthority {
    const obj = strictObject(value, path, ['schema_version', 'channel', 'components', 'targets']);
    if (obj.schema_version !== 1) throw new Error(`${path}.schema_version must be 1`);
    return {
        schemaVersion: 1,
        channel: exactString(obj.channel, 'nightly-2026-08-25', `${path}.channel`),
        components: exactStringArray(
            obj.components,
            `${path}.components`,
            ['rust-src', 'rustc-codegen-cranelift-preview'],
        ),
        targets: exactStringArray(obj.targets, `${path}.targets`, ['wasm32-unknown-unknown']),
    };
}

export function parseBuildToolAuthority(value: unknown, path: string): BuildToolAuthority {
    const obj = strictObject(value, path, ['version', 'authority_sha256']);
    const version = boundedString(obj.version, `${path}.version`, 128);
    const components = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u.exec(version);
    if (components === null || components.slice(1).some(component =>
        BigInt(component ?? '0') > 0xffff_ffff_ffff_ffffn)) {
        throw new Error(`${path}.version must be an exact stable semantic version`);
    }
    return {
        version,
        authoritySha256: nonzeroSha256(obj.authority_sha256, `${path}.authority_sha256`),
    };
}

export function parseViewerArtifactClosure(value: unknown, path: string): readonly NamedArtifact[] {
    const artifacts = array(value, path).map((item, index) => parseNamedArtifact(item, `${path}[${index}]`));
    if (artifacts.length < 2 || artifacts.length > 64 || !strictlySorted(artifacts.map(a => a.path))) {
        throw new Error(`${path} must contain 2–64 artifacts in strict path order`);
    }
    const roles = artifacts.map(({ role }) =>
        role.kind === 'auxiliary' || role.kind === 'java_script_module'
            ? `${role.kind}:${role.name}`
            : role.kind);
    const javaScriptModuleNames = artifacts.flatMap(({ role }) =>
        role.kind === 'java_script_module' ? [role.name] : []);
    if (new Set(roles).size !== roles.length
        || new Set(javaScriptModuleNames).size !== javaScriptModuleNames.length
        || roles.filter(role => role === 'entry_java_script').length !== 1
        || roles.filter(role => role === 'web_assembly').length !== 1) {
        throw new Error(`${path} must have unique roles and exactly one JS entry and WASM`);
    }
    return artifacts;
}

export function parseBrowserOriginClosure(value: unknown, path: string): readonly BrowserPagesArtifact[] {
    const artifacts = array(value, path).map((item, index) => {
        const itemPath = `${path}[${index}]`;
        const obj = strictObject(item, itemPath, ['path', 'artifact']);
        return {
            path: viewerArtifactPath(obj.path, `${itemPath}.path`),
            artifact: parseArtifactRef(obj.artifact, `${itemPath}.artifact`),
        };
    });
    if (artifacts.length === 0 || artifacts.length > 512 || !strictlySorted(artifacts.map(a => a.path))) {
        throw new Error(`${path} must contain 1–512 artifacts in strict path order`);
    }
    return artifacts;
}

export function parseReplayArtifact(value: unknown): ReplayArtifact {
    const obj = strictObject(value, 'replay_artifact', ['artifact', 'replay_schema_version']);
    const artifact = parseArtifactRef(obj.artifact, 'replay_artifact.artifact');
    if (artifact.mediaType !== 'application/x-robin-rhrec+compact') {
        throw new Error('replay_artifact must use the canonical CompactRhrec media type');
    }
    const replaySchemaVersion = u32Positive(
        obj.replay_schema_version,
        'replay_artifact.replay_schema_version',
    );
    if (replaySchemaVersion !== CURRENT_RANKED_REPLAY_SCHEMA_VERSION) {
        throw new Error(`replay_artifact.replay_schema_version must be ${CURRENT_RANKED_REPLAY_SCHEMA_VERSION}`);
    }
    return {
        artifact,
        replaySchemaVersion,
    };
}

export async function parseAndVerifyBuildManifest(value: unknown, expectedSha256: string): Promise<DigestVerified<BuildManifest>> {
    const parsed = parseBuildManifest(value);
    return await verifyCanonicalDocument(value, parsed, expectedSha256, 'build_manifest');
}

export function parseArtifactRef(value: unknown, path: string): ArtifactRef {
    const obj = strictObject(value, path, ['sha256', 'byte_length', 'media_type']);
    return {
        sha256: nonzeroSha256(obj.sha256, `${path}.sha256`),
        byteLength: positiveInteger(obj.byte_length, `${path}.byte_length`),
        mediaType: boundedString(obj.media_type, `${path}.media_type`, 128),
    };
}

export function parseCampaignArtifactRef(value: unknown, path: string): ArtifactRef {
    const artifact = parseArtifactRef(value, path);
    if (artifact.mediaType !== 'application/x-robin-campaign+bitcode') {
        throw new Error(`${path} has the wrong ranked-campaign media_type`);
    }
    return artifact;
}

export function parseNamedArtifact(value: unknown, path: string): NamedArtifact {
    const obj = strictObject(value, path, ['path', 'role', 'artifact']);
    const roleObj = object(obj.role, `${path}.role`);
    const kind = enumeration(
        roleObj.kind,
        ['entry_java_script', 'java_script_module', 'web_assembly', 'auxiliary'] as const,
        `${path}.role.kind`,
    );
    let role: ViewerArtifactRole;
    if (kind === 'auxiliary' || kind === 'java_script_module') {
        assertExactKeys(roleObj, `${path}.role`, ['kind', 'name']);
        role = { kind, name: boundedString(roleObj.name, `${path}.role.name`, 128) };
    } else {
        assertExactKeys(roleObj, `${path}.role`, ['kind']);
        role = { kind };
    }
    const artifact = parseArtifactRef(obj.artifact, `${path}.artifact`);
    const artifactPath = viewerArtifactPath(obj.path, `${path}.path`);
    const isJavaScriptPath = artifactPath.endsWith('.js');
    const isWebAssemblyPath = artifactPath.endsWith('.wasm');
    const hasExecutablePathExtension = /\.(?:c?js|mjs|jsx|wasm)$/iu.test(artifactPath);
    if ((kind === 'entry_java_script' || kind === 'java_script_module')
        && artifact.mediaType !== 'text/javascript') {
        throw new Error(`${path} JavaScript artifacts must use text/javascript`);
    }
    if ((kind === 'entry_java_script' || kind === 'java_script_module') && !isJavaScriptPath) {
        throw new Error(`${path} JavaScript artifacts must use a canonical .js path`);
    }
    if (kind === 'web_assembly' && artifact.mediaType !== 'application/wasm') {
        throw new Error(`${path} WASM artifact must use application/wasm`);
    }
    if (kind === 'web_assembly' && !isWebAssemblyPath) {
        throw new Error(`${path} WASM artifacts must use a canonical .wasm path`);
    }
    if (kind === 'auxiliary' && isExecutableMediaType(artifact.mediaType)) {
        throw new Error(`${path} auxiliary artifacts must not use an executable media type`);
    }
    if (kind === 'auxiliary' && hasExecutablePathExtension) {
        throw new Error(`${path} auxiliary artifacts must not use an executable path extension`);
    }
    if (role.kind === 'java_script_module' && artifactPath !== `viewer/${role.name}`) {
        throw new Error(`${path} JavaScript module name does not match its authenticated viewer path`);
    }
    return { path: artifactPath, role, artifact };
}

export function isExecutableMediaType(mediaType: string): boolean {
    return mediaType === 'application/wasm'
        || /^(?:application|text)\/(?:java|ecma)script(?:$|;)/iu.test(mediaType);
}
