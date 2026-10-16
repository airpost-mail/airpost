import type { FileJSON, MessageJSON } from '@/engine';

/** Files to list under a message: everything except inline parts the body references. */
export function listedAttachments(m: MessageJSON): FileJSON[] {
  return m.files.filter((f) => !f.contentId || !m.body?.includes(`cid:${f.contentId}`));
}
