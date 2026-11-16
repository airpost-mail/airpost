TS libraries shared by apps, e.g. protocol types generated from engine structs.

Each package ships TypeScript source (`main: "src/index"`) and is linked into an app with a
`file:` dependency, e.g. `"@airpost/email-dark": "file:../../packages/email-dark"` in
`apps/desktop/package.json`. The desktop compiles it like its own code: on the fly in dev
and specs, and in `build/build.js` (`node_modules/@airpost/*/src`) for packaged builds. The web
(Vite) compiles it as source too; `apps/web/vite.config.ts` lets the dev server read it.

A packaged desktop build ships only a package's `package.json` and the top-level entries its
`files` lists (`src`, plus `node` for `@airpost/client/node`), never its own `node_modules`, which
holds its dev tooling. A runtime dependency must also be in `apps/desktop`'s `dependencies`,
where Node finds it from inside the asar. `build/build.js` refuses a package without `files` or
with a runtime dependency the desktop lacks, and fails if the asar carries anything else of
`node_modules/@airpost/*`.

- `client`: the shared engine client: the protocol's TypeScript types and constants, generated
  from `crates/airpost-ipc`, the JSON-RPC core, the session, the WebSocket transport (and the Unix
  socket one in `@airpost/client/node`), the settings cache and task waits.
- `email-dark`: renders HTML email in a dark colour scheme.
