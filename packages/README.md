TS libraries shared by apps, e.g. protocol types generated from engine structs.

Each package ships TypeScript source (`main: "src/index"`) and is linked into an app with a
`file:` dependency, e.g. `"@tern/email-dark": "file:../../packages/email-dark"` in
`apps/desktop/package.json`. The desktop compiles it like its own code: on the fly in dev
and specs, and in `build/build.js` (`node_modules/@tern/*/src`) for packaged builds. The web
(Vite) compiles it as source too; `apps/web/vite.config.ts` lets the dev server read it.

- `client`: the shared engine client: the protocol's TypeScript types and constants, generated
  from `crates/tern-ipc`, the JSON-RPC core, the session, the WebSocket transport (and the Unix
  socket one in `@tern/client/node`), the settings cache and task waits.
- `email-dark`: renders HTML email in a dark colour scheme.
