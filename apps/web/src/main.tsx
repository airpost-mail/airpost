import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider } from '@/components/ThemeProvider';
import { createEngineClient, EngineProvider } from '@/engine';
import { getSessionToken, subscribeSession } from '@/features/auth';
import { createQueryClient } from './queryClient';
import { createAppRouter } from './router';
import './index.css';
import '@/lib/preferences';

const queryClient = createQueryClient();
const engine = createEngineClient({ getToken: getSessionToken });
// A new token means a new identity on the socket; the next query reconnects with it.
let token = getSessionToken();
subscribeSession(() => {
  const next = getSessionToken();
  if (next === token) return;
  token = next;
  engine.close();
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
