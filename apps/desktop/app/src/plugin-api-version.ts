/**
 * The version of the plugin API, matched as a semver range against a plugin's
 * `engines.mailspring`. It moves with the API, not the app: a minor bump for additions, a
 * major one only when a deprecated export is removed (docs/desktop-client-plan.md section 6).
 */
export const PLUGIN_API_VERSION = '1.26.0';
