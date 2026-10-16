import { useLocation, useNavigate } from '@tanstack/react-router';
import { useEffect, useRef } from 'react';
import { getSignOutNotice, useSessionSnapshot } from './session';
import { useEndSessionCleanup } from './use-sign-out';

/**
 * Mount inside the paired layout. `requireSession` only runs on navigation; this catches a
 * pairing that ends while a page is open (the gateway answered 401, or closed the socket
 * with 4401 after a revoke) and sends the user to /pair, back to the same place afterwards.
 */
export function SessionGuard() {
  const { status } = useSessionSnapshot();
  const href = useLocation({ select: (l) => l.href });
  const navigate = useNavigate();
  const cleanup = useEndSessionCleanup();
  // The guard stays mounted until /pair renders; the location change in between mustn't
  // send a second navigation with /pair as the place to come back to.
  const sent = useRef(false);

  useEffect(() => {
    if (status !== 'unpaired') {
      sent.current = false;
      return;
    }
    if (sent.current) return;
    sent.current = true;
    // A deliberate sign-out starts fresh; anything else returns to this page after pairing.
    const search = getSignOutNotice() === 'user' ? {} : { redirect: href };
    void navigate({ to: '/pair', search, replace: true }).then(cleanup);
  }, [status, href, navigate, cleanup]);

  return null;
}
