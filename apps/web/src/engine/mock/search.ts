// A small subset of the desktop's search grammar (SearchQueryParser): free text plus
// from:, to:, subject:, is:unread|read|starred, in:<role or name>, has:attachment.
// TODO(engine): the real `search` takes none of these operators. It reads the query as plain
// text (every term quoted for FTS5), so `from:jonas` only matches the literal words. This
// grammar is what the client sends; the engine has to grow it (see features/search/query.ts).

export interface ParsedQuery {
  text: string[];
  from: string[];
  to: string[];
  subject: string[];
  is: string[];
  in: string[];
  has: string[];
}

const FIELDS = ['from', 'to', 'subject', 'is', 'in', 'has'] as const;
type Field = (typeof FIELDS)[number];

export function parseQuery(query: string): ParsedQuery {
  const out: ParsedQuery = { text: [], from: [], to: [], subject: [], is: [], in: [], has: [] };
  const re = /(\w+):(?:"([^"]*)"|(\S+))|"([^"]*)"|(\S+)/g;
  for (const m of query.matchAll(re)) {
    const [, field, quotedValue, value, quoted, word] = m;
    if (field && (FIELDS as readonly string[]).includes(field.toLowerCase())) {
      out[field.toLowerCase() as Field].push((quotedValue ?? value ?? '').toLowerCase());
    } else if (field) {
      out.text.push(`${field}:${quotedValue ?? value}`.toLowerCase());
    } else {
      out.text.push((quoted ?? word ?? '').toLowerCase());
    }
  }
  return out;
}
