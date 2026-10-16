// Command matching for the palette. cmdk's own filter is off because mail results come
// from the engine and must not be re-filtered client-side.

export interface PaletteCommand {
  id: string;
  group: 'Actions' | 'Go to';
  label: string;
  /** Muted text after the label (e.g. the account of a folder). */
  detail?: string;
  keywords?: string[];
  shortcut?: string;
  /** Hidden until the user types (e.g. user labels, to keep the idle list short). */
  onlyWhenSearching?: boolean;
  run: () => void;
}

function words(s: string) {
  return s
    .toLowerCase()
    .split(/[\s/._-]+/)
    .filter(Boolean);
}

/**
 * 0 means no match. 3: the label starts with the query; 2: every query word starts a word
 * of the label or keywords; 1: the query occurs somewhere in the label.
 */
export function matchScore(query: string, label: string, keywords: string[] = []): number {
  const q = query.trim().toLowerCase();
  if (!q) return 1;
  const l = label.toLowerCase();
  if (l.startsWith(q)) return 3;
  const hay = [...words(label), ...keywords.flatMap(words)];
  if (words(q).every((w) => hay.some((h) => h.startsWith(w)))) return 2;
  if (l.includes(q)) return 1;
  return 0;
}

/** Commands matching `query`, best first; stable within a score. */
export function filterCommands<T extends PaletteCommand>(commands: T[], query: string): T[] {
  const searching = query.trim().length > 0;
  return commands
    .filter((c) => searching || !c.onlyWhenSearching)
    .map((c, i) => ({ c, i, s: matchScore(query, c.label, c.keywords) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .map((x) => x.c);
}
