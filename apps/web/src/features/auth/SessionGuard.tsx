import { useLocation, useNavigate } from '@tanstack/react-router';
import { useEffect, useRef } from 'react';
import { getSignOutNotice, useSession } from './session';
import { useEndSessionCleanup } from './use-sign-out';

/**
 * Mount inside the signed-in layout. `requireSession` only runs on navigation; this catches a
 * session that ends while a page is open (another tab signed out, or the server revoked the
 * token) and sends the user to /login, back to the same place afterwards.
 */
export function SessionGuard() {
  const session = useSession();
  const href = useLocation({ select: (l) => l.href });
  const navigate = useNavigate();
  const cleanup = useEndSessionCleanup();
  const hadSession = useRef(session !== null);

  useEffect(() => {
    if (session) {
      hadSession.current = true;
      return;
    }
    if (!hadSession.current) return;
    hadSession.current = false;
    // A deliberate sign-out starts fresh; anything else returns to this page after sign-in.
    const search = getSignOutNotice() === 'user' ? {} : { redirect: href };
    void navigate({ to: '/login', search, replace: true }).then(cleanup);
  }, [session, href, navigate, cleanup]);

  return null;
}
