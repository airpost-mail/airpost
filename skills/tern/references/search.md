# Tern search syntax

`search_mail` (MCP) and `tern search` (CLI) take the same query as Tern's search bar. The
engine parses it; a query it can't read comes back as an error with the reason.

## Terms

- Plain words must all match somewhere in the thread (sender, recipients, subject, folder
  names or body), each as the start of a word: `invo` matches "invoice".
- `"quoted phrases"` match as a phrase.
- Terms next to each other are combined with AND. `AND` may also be written out.

## Fields

| Field | Example |
| --- | --- |
| `from:` | `from:alice`, `from:alice@example.com` |
| `to:` | `to:team@example.com` |
| `subject:` | `subject:"weekly sync"` |
| `in:` | `in:inbox`, `in:sent`, `in:drafts`, `in:trash`, `in:spam`, `in:archive`, or a folder/label name or path |
| `is:` | `is:unread`, `is:read`, `is:starred`, `is:unstarred` |
| `has:` | `has:attachment` |
| `after:` / `since:` | includes that day |
| `before:` | excludes that day |

Dates: `2026-05-31`, `2026/05/31`, `"may 31 2026"`, month or weekday names, `today`,
`yesterday`, `"3 days ago"`, `3d`, `2w`, `6m`, `1y`, `"last week"`, `"last month"`, `"last
year"`. They are read in the user's time zone. Quote values that contain spaces.

Keywords and `AND`/`OR`/`NOT` match in any case. A word is a field only when a `:` follows
it, so `trip to paris` is three words.

## Combining

- `a OR b`: either.
- `a NOT b`: a, but not b. NOT takes everything after it up to a `)` or `OR`. Put it
  between terms: `budget NOT in:trash`.
- Parentheses group: `(from:bob OR from:carol) has:attachment`.

## Not supported

There is no `label:`, `cc:`, `bcc:`, `larger:`, `filename:` or `-term` negation, and `is:`
only takes the four values above. Use `in:<label name>` for a Gmail label and `NOT` for
negation. To list snoozed mail, call `list_threads` on the folder whose `role` is
`snoozed`. Any other `is:` or `has:` value, empty parentheses or a trailing
`AND`/`OR`/`NOT` is an error.

## Examples

```
from:alice is:unread
in:inbox is:unread after:yesterday
subject:invoice has:attachment after:2026-01-01 before:2026-04-01
(from:bob OR from:carol) "project alpha"
in:sent to:dana@example.com after:2w
newsletter NOT in:trash
```

Results are thread summaries, newest first by `lastMessageReceivedTimestamp`.
