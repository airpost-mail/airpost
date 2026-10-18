/**
 * The version of the plugin API, matched as a semver range against a plugin's
 * `engines.mailspring`. It moves with the API, not the app: a minor bump for additions, a
 * major one only when a deprecated export is removed (docs/desktop-client-plan.md section 6).
 *
 * 1.27.0: `Engine` (src/engine-api.ts) and `tern.engineMethods`;
 * MessageViewExtension.renderedMessageBodyIntoDocument gets `body` (MessageBody) instead of the
 * document, which is in a cross-origin frame now.
 *
 * 1.28.0: `AI` (src/ai.ts), the AI settings, and the `ai-summary` and `ai-reply` capabilities.
 *
 * 1.29.0: the `ai-triage` capability and the `ai.triage.*` types.
 */
export const PLUGIN_API_VERSION = '1.29.0';
