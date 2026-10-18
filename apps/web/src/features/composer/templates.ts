// Message templates. The desktop keeps them as HTML files in its config directory; the web
// client keeps them in localStorage until the engine has somewhere to put them.
// TODO(engine): the engine keeps `templates` (DCP stage 3), but a device without the admin
// scope can't set them (ms-ipc `RELAYED_SETTINGS_KEYS`: HTML the desktop renders) and the
// desktop doesn't mirror its template files there; both need deciding before the web moves.
import { generateId } from '@/engine';
import { localStore } from './local-store';

export interface Template {
  id: string;
  name: string;
  subject: string;
  /** Sanitised editor HTML. */
  body: string;
}

const store = localStore<Template[]>('tern.composer.templates', []);

export const useTemplates = store.use;

export function saveTemplate(t: Omit<Template, 'id'>): Template {
  const created = { ...t, id: generateId() };
  store.set([...store.get().filter((x) => x.name !== t.name), created]);
  return created;
}

export function deleteTemplate(id: string) {
  store.set(store.get().filter((t) => t.id !== id));
}
