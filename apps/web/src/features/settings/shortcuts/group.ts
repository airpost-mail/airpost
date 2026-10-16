import type { ShortcutDef, ShortcutGroup } from '@/lib/shortcuts';

const ORDER: ShortcutGroup[] = ['Navigation', 'Thread', 'Compose', 'Search', 'Application'];

/** Groups the registry, listing each description once (several views may bind the same). */
export function groupShortcuts(list: ShortcutDef[], filter = ''): [ShortcutGroup, ShortcutDef[]][] {
  const q = filter.trim().toLowerCase();
  const groups = new Map<ShortcutGroup, ShortcutDef[]>();
  for (const s of list) {
    const keys = Array.isArray(s.keys) ? s.keys : [s.keys];
    if (q && !s.description.toLowerCase().includes(q) && !keys.some((k) => k.includes(q))) continue;
    const g = s.group ?? 'Application';
    const bucket = groups.get(g) ?? [];
    if (!bucket.some((x) => x.description === s.description)) bucket.push(s);
    groups.set(g, bucket);
  }
  return ORDER.filter((g) => groups.has(g)).map((g) => [g, groups.get(g)!]);
}
