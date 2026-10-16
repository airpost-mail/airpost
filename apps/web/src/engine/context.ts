import { createContext, useContext } from 'react';
import type { EngineClient } from './client';

export const EngineContext = createContext<EngineClient | null>(null);

export function useEngine(): EngineClient {
  const client = useContext(EngineContext);
  if (!client) throw new Error('useEngine must be used inside <EngineProvider>');
  return client;
}
