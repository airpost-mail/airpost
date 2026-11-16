# Default Profile

Template for a new Airpost profile (`~/Library/Application Support/Airpost` on
macOS, the matching config directory on Windows and Linux).
`src/config/persistence-manager.ts` writes `config.json` into a
profile that has none, and copies the whole folder only when the profile
directory doesn't exist yet.
