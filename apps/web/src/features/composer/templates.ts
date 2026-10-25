// The engine's message templates (`templates.list|get`): HTML files in its config directory,
// the ones the desktop, the CLI and MCP use. Saving and deleting them stays on the engine's
// machine (`templates.save|delete` are not relayed), so the web lists and inserts them.
import { useQuery } from '@tanstack/react-query';
import { ensureConnected, useEngine, type EngineClient, type TemplateInfo } from '@/engine';
import { sanitizeEditorHtml } from './html';

export type { TemplateInfo };

export function useTemplates() {
  const client = useEngine();
  return useQuery({
    queryKey: ['engine', 'templates'],
    queryFn: async () => {
      await ensureConnected(client);
      return (await client.call('templates.list')).templates;
    },
  });
}

/** A template's subject and body, the body as editor HTML. */
export async function loadTemplate(
  client: EngineClient,
  name: string,
): Promise<{ subject: string; body: string }> {
  await ensureConnected(client);
  const { template } = await client.call('templates.get', { name });
  return { subject: template.subject, body: sanitizeEditorHtml(template.body ?? '') };
}
