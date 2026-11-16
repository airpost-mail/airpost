// Which engine the app talks to: the in-browser mock, or a real engine through the gateway
// that served this page.

export type EngineMode = 'mock' | 'gateway';

/** Dev builds only: `localStorage.setItem('airpost.engine', 'gateway')` overrides the build. */
export const ENGINE_MODE_KEY = 'airpost.engine';

function parse(v: unknown): EngineMode | null {
  return v === 'mock' || v === 'gateway' ? v : null;
}

export function engineMode(): EngineMode {
  if (import.meta.env.DEV) {
    try {
      const v = parse(localStorage.getItem(ENGINE_MODE_KEY));
      if (v) return v;
    } catch {
      // Storage blocked: use the build default.
    }
  }
  return parse(import.meta.env.VITE_AIRPOST_ENGINE) ?? 'mock';
}

/** The gateway's API, on the page's own origin. */
export const GATEWAY_BASE = '/gateway/v1';

export function gatewayWsUrl(loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  return `${loc.protocol === 'https:' ? 'wss' : 'ws'}://${loc.host}${GATEWAY_BASE}/ws`;
}
