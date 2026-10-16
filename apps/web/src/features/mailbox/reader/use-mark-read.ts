import { useEffect, useRef } from 'react';
import { tasks, useQueueTask, type ThreadJSON } from '@/engine';
import { usePreferences } from '@/lib/preferences';

/**
 * Marks the thread read once per open, after the `markReadDelayMs` preference (-1 means
 * never). Leaving before the delay keeps it unread; a later "mark unread" must stick.
 */
export function useMarkReadOnOpen(thread: ThreadJSON | null | undefined) {
  const { mutate } = useQueueTask();
  const { markReadOnOpen, markReadDelayMs } = usePreferences();
  const handled = useRef(false);
  const unread = !!thread && thread.unread > 0;
  const aid = thread?.aid;
  const id = thread?.id;
  const manual = !markReadOnOpen || markReadDelayMs < 0;

  useEffect(() => {
    if (handled.current || !aid || !id) return;
    if (!unread || manual) {
      handled.current = true;
      return;
    }
    const markRead = () => {
      handled.current = true;
      mutate(
        tasks.changeUnread({ accountId: aid, threadIds: [id], unread: false, source: 'open' }),
      );
    };
    if (markReadDelayMs === 0) return void markRead();
    const timer = setTimeout(markRead, markReadDelayMs);
    return () => clearTimeout(timer);
  }, [aid, id, unread, manual, markReadDelayMs, mutate]);
}
