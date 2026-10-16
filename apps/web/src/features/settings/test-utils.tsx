import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { EngineProvider, MockEngine } from '@/engine';

/** Renders inside a fresh query client and a zero-latency MockEngine. */
export function renderWithEngine(ui: ReactElement) {
  const engine = new MockEngine({ latency: 0, now: 1_780_000_000 });
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const result = render(
    <QueryClientProvider client={qc}>
      <EngineProvider client={engine}>{ui}</EngineProvider>
    </QueryClientProvider>,
  );
  return { ...result, engine, qc };
}
