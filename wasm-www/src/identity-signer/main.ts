import { installMultiplayerIdentitySigner } from '../multiplayer_identity_protocol.js';
import {
    installLeaderboardIdentitySigner,
} from './protocol.js';
import { startIdentitySigner, type InitializedBridge } from './startup.js';

// This document is deployed only on the isolated signer origin. The protocol
// implementations refuse top-level use, accept messages only from the exact
// production game origin, and expose only their domain-bound operations.
void startIdentitySigner({
    load: async url => await import(/* @vite-ignore */ url) as InitializedBridge,
    leaderboard: installLeaderboardIdentitySigner,
    multiplayer: installMultiplayerIdentitySigner,
});
