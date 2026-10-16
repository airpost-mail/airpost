/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `gateway` for the bundle the gateway serves; anything else selects the in-browser mock. */
  readonly VITE_TERN_ENGINE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
