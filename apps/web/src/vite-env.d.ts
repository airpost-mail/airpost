/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** WebSocket URL of a hosted engine; unset selects the in-browser mock. */
  readonly VITE_ENGINE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
