// The desktop half of the search grammar parity suite (docs/desktop-client-plan.md stage 9).
//
// Writes crates/ms-store/tests/fixtures/search_queries.json: for each query text below, the
// AST `SearchQueryParser.parse` builds (or that it fails), and for queries without dates the
// WHERE clause `LocalSearchQueryBackend` compiles for Thread. The engine's parser must build
// the same AST, and `crates/ms-store/tests/search_grammar_parity.rs` runs both WHERE clauses
// on one fixture database and compares the threads. The texts avoid the places where the
// engine's grammar differs on purpose (see crates/ms-store/src/search_grammar.rs).
//
// After changing the parser or the list, regenerate the file with
//   TERN_WRITE_QUERY_FIXTURES=1 npm test -- --spec-file-pattern=search-query-capture
// and run `cargo test -p ms-store --test search_grammar_parity`.
import fs from 'fs';
import path from 'path';
import { SearchQueryParser } from '../src/services/search/search-query-parser';
import LocalSearchQueryBackend from '../src/services/search/search-query-backend-local';
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
} from '../src/services/search/search-query-ast';

const REPO = path.resolve(__dirname, '..', '..', '..');
const FIXTURE = path.join(REPO, 'crates', 'ms-store', 'tests', 'fixtures', 'search_queries.json');
const WRITE = !!process.env.TERN_WRITE_QUERY_FIXTURES;

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

function capture() {
  const queries = QUERIES.map((text) => {
    let ast: Json;
    let where: string | null = null;
    try {
      const parsed = SearchQueryParser.parse(text);
      ast = toJSON(parsed);
      if (!JSON.stringify(ast).includes('"date"')) {
        where = new LocalSearchQueryBackend('Thread').compile(parsed);
      }
    } catch (e) {
      return { text, error: true };
    }
    return { text, ast, where };
  });
  return { generatedBy: 'apps/desktop/spec/search-query-capture-spec.ts', queries };
}

describe('Search grammar capture for the engine', () => {
  it('matches the captured queries', () => {
    const text = `${JSON.stringify(capture(), null, 2)}\n`;
    if (WRITE) fs.writeFileSync(FIXTURE, text);
    const onDisk = fs.existsSync(FIXTURE) ? fs.readFileSync(FIXTURE, 'utf8') : '';
    if (onDisk !== text) {
      throw new Error(
        `${path.relative(REPO, FIXTURE)} is out of date; regenerate it with TERN_WRITE_QUERY_FIXTURES=1`
      );
    }
  });
});
