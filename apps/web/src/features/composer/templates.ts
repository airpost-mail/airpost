// Message templates. The desktop keeps them as HTML files in its config directory; the web
// client keeps them in localStorage until the engine has somewhere to put them.
// TODO(engine): templates move to engine settings (`templates`, DCP stage 3) so they follow
// the user across browsers and the desktop app.
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
