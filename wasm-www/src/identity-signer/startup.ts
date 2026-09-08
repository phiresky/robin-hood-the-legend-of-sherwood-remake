import type { LeaderboardIdentityBridgeModule } from './protocol.js';

export type InitializedBridge = LeaderboardIdentityBridgeModule & {
    readonly default: (options: { readonly module_or_path: string }) => Promise<unknown>;
};

/** Install no request handlers until the typed wasm bridge has initialized successfully. */
export async function startIdentitySigner(deps: {
    load: (url: string) => Promise<InitializedBridge>;
    leaderboard: (bridge: LeaderboardIdentityBridgeModule) => void;
    multiplayer: () => void;
}): Promise<void> {
    const bridge = await deps.load('/identity-signer/bridge/leaderboard_identity_bridge.js');
    await bridge.default({ module_or_path: '/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm' });
    deps.leaderboard(bridge);
    deps.multiplayer();
}
