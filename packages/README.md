TS libraries shared by apps, e.g. protocol types generated from engine structs.

Each package ships TypeScript source (`main: "src/index"`) and is linked into an app with a
`file:` dependency, e.g. `"@tern/email-dark": "file:../../../packages/email-dark"` in
`apps/desktop/package.json`. The desktop compiles it like its own code: on the fly in dev
and specs, and in `build/build.js` (`node_modules/@tern/*/src`) for packaged builds.

- `email-dark`: renders HTML email in a dark colour scheme.
