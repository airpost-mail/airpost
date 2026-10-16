import { useQuery } from '@tanstack/react-query';
import { authTransport } from './session';

/** `GET /tern/v1/server` for the URL in the login form; Sign in waits for a good answer. */
export function useServerProbe(url: string | null) {
  return useQuery({
    queryKey: ['auth', 'server', url],
    queryFn: () => authTransport.server(url!),
    enabled: !!url,
    retry: false,
    staleTime: 60_000,
  });
}
