import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider } from '@/components/ThemeProvider';
import { createEngineClient, EngineProvider } from '@/engine';
import { markUnpaired, refreshSession } from '@/features/auth';
import { createQueryClient } from './queryClient';
import { createAppRouter } from './router';
import './index.css';
import '@/lib/preferences';

const queryClient = createQueryClient();
// The gateway's session route is the client's health check, which also keeps the session
// store current; the pairing cookie authorises the socket itself.
const engine = createEngineClient({
  health: refreshSession,
  onUnpaired: () => markUnpaired('revoked'),
});
const router = createAppRouter({ queryClient, engine });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <EngineProvider client={engine}>
          <RouterProvider router={router} />
        </EngineProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </StrictMode>,
);
