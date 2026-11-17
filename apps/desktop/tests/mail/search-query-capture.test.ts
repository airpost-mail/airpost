// The desktop half of the search grammar parity suite.
//
// Writes crates/airpost-store/tests/fixtures/search_queries.json: for each query text below, the
// AST `SearchQueryParser.parse` builds (or that it fails), and for queries without dates the
// WHERE clause the desktop's SQL backend compiled for Thread. The engine's parser must build
// the same AST, and `crates/airpost-store/tests/search_grammar_parity.rs` runs both WHERE clauses
// on one fixture database and compares the threads. The texts avoid the places where the
// engine's grammar differs on purpose (see crates/airpost-store/src/search_grammar.rs).
//
// The desktop sends the grammar to the engine since 2026-10-25 and its SQL backend is gone, so
// the WHERE clauses are kept as last generated; a query added here has none.
//
// After changing the parser or the list, regenerate the file with
//   AIRPOST_WRITE_QUERY_FIXTURES=1 npm test -- --spec-file-pattern=search-query-capture
// and run `cargo test -p airpost-store --test search_grammar_parity`.
import fs from 'fs';
import path from 'path';
import { SearchQueryParser } from '../../src/mail/search/parser';
import {
  AndQueryExpression,
  DateQueryExpression,
  FromQueryExpression,
  GenericQueryExpression,
  HasAttachmentQueryExpression,
  InQueryExpression,
  NotQueryExpression,
  OrQueryExpression,
  QueryExpression,
  StarredStatusQueryExpression,
  SubjectQueryExpression,
  ToQueryExpression,
  UnreadStatusQueryExpression,
} from '../../src/mail/search/ast';

const REPO = path.resolve(__dirname, '..', '..', '..', '..');
const FIXTURE = path.join(
  REPO,
  'crates',
  'airpost-store',
  'tests',
  'fixtures',
  'search_queries.json'
);
const WRITE = !!process.env.AIRPOST_WRITE_QUERY_FIXTURES;

const QUERIES = [
  // search-query-parser-spec.ts
  'blah',
  '"foo bar"',
  'to:blah',
  'from:blah',
  'subject:blah',
  'to:mhahnenb@gmail.com',
  'to:"mhahnenb@gmail.com"',
  'to:"Mark mhahnenb@gmail.com"',
  'is:unread',
  'is:read',
  'is:starred',
  'is:unstarred',
  'in:foo',
  'has:attachment',
  'to:to',
  'to:subject',
  'to:from',
  'to:unread',
  'to:starred',
  'foo bar',
  'foo AND bar',
  'foo OR bar',
  'to:foo OR bar',
  'foo OR to:bar',
  'foo bar baz',
  'foo AND bar AND baz',
  'foo OR bar AND baz',
  'foo OR bar OR baz',
  'foo is:unread',
  'is:unread foo',
  // The call sites: search-bar-util.ts getThreadSuggestions, sidebar-related-threads.tsx.
  'subject:"budget" NOT (in:trash OR in:spam)',
  'from:ann@x.test',
  // Against the parity fixture's threads.
  'budget',
  'budg',
  'lisbon trip',
  '"lisbon trip"',
  'from:ann',
  'From:Ann',
  'to:bob',
  'in:inbox',
  'in:sent is:unread',
  'IS:UNREAD',
  'has:attachments',
  'has:attachment from:ann',
  'is:starred OR has:attachment',
  'budget NOT in:trash',
  'budget AND NOT in:trash',
  '(budget OR lisbon) is:unread',
  'is:unread NOT is:starred',
  'from:ann OR from:bob',
  'subject:budget OR is:starred',
  'is:read in:inbox',
  'subject:"q3 budget"',
  'in:sent from:me@x.test',
  'is:unread NOT hello AND in:inbox',
  'invoice OR (from:carol has:attachment)',
  'subject:lisbon OR subject:budget NOT is:unread',
  'zzz',
  // Dates: the AST only; the engine resolves them on its own terms.
  'after:2024-01-01',
  'before:2024/01/01 from:ann',
  'since:yesterday',
  'after:"3 days ago" budget',
  // Both fail.
  'from:',
  '"unterminated',
  '(foo',
  'foo OR',
  'foo AND',
  'foo NOT',
];

type Json = Record<string, unknown>;

function toJSON(e: QueryExpression): Json {
  const pair = (type: string, n: { e1: QueryExpression; e2: QueryExpression }) => ({
    type,
    e1: toJSON(n.e1),
    e2: toJSON(n.e2),
  });
  if (e instanceof AndQueryExpression) return pair('and', e);
  if (e instanceof OrQueryExpression) return pair('or', e);
  if (e instanceof NotQueryExpression) return pair('not', e);
  if (e instanceof FromQueryExpression) return { type: 'from', text: e.text.token.s };
  if (e instanceof ToQueryExpression) return { type: 'to', text: e.text.token.s };
  if (e instanceof SubjectQueryExpression) return { type: 'subject', text: e.text.token.s };
  if (e instanceof InQueryExpression) return { type: 'in', text: e.text.token.s };
  if (e instanceof GenericQueryExpression) return { type: 'generic', text: e.text.token.s };
  if (e instanceof UnreadStatusQueryExpression) return { type: 'unread', status: e.status };
  if (e instanceof StarredStatusQueryExpression) return { type: 'starred', status: e.status };
  if (e instanceof HasAttachmentQueryExpression) return { type: 'hasAttachment' };
  if (e instanceof DateQueryExpression) {
    return { type: 'date', text: e.text.token.s, direction: e.direction };
  }
  throw new Error(`no JSON form for ${e}`);
}

function capture(onDisk: string) {
  const captured: { text: string; where?: string | null }[] = onDisk
    ? JSON.parse(onDisk).queries
    : [];
  const queries = QUERIES.map((text) => {
    let ast: Json;
    const where = captured.find((q) => q.text === text)?.where ?? null;
    try {
      ast = toJSON(SearchQueryParser.parse(text));
    } catch (e) {
      return { text, error: true };
    }
    return { text, ast, where };
  });
  return { generatedBy: 'apps/desktop/tests/mail/search-query-capture.test.ts', queries };
}

describe('Search grammar capture for the engine', () => {
  it('matches the captured queries', () => {
    const onDisk = fs.existsSync(FIXTURE) ? fs.readFileSync(FIXTURE, 'utf8') : '';
    const text = `${JSON.stringify(capture(onDisk), null, 2)}\n`;
    if (WRITE) {
      fs.writeFileSync(FIXTURE, text);
      return;
    }
    if (onDisk !== text) {
      throw new Error(
        `${path.relative(REPO, FIXTURE)} is out of date; regenerate it with AIRPOST_WRITE_QUERY_FIXTURES=1`
      );
    }
  });
});
