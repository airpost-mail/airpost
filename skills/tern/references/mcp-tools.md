# Tern MCP tools

Every tool of the `tern` MCP server (`tern-mcp`), its arguments (`*` = required) and the
access level it needs. Errors come back as a tool error whose text is `{"error": "..."}`.

Times are ISO 8601. Without an offset they are local time; a date alone is midnight UTC.

## Read (any level)

| Tool | Arguments | Returns |
| --- | --- | --- |
| `list_accounts` | none | `[{id, name, email, provider, usesLabels}]` for every visible account |
| `list_folders` | `accountId`* | `[{id, name, role, path}]`; `role` is `inbox`, `sent`, `drafts`, `trash`, `spam`, `archive`, `all`, `important`, `snoozed` or null |
| `search_mail` | `query`*, `accountId`, `limit` (150), `offset` (0) | thread summaries, newest first (see search.md) |
| `list_threads` | `accountId`*, `folderId`*, `limit` (25), `offset`, `unread` | thread summaries |
| `get_thread` | `threadId`* | `{id, subject, unread, starred, accountId, categories, messages: [...]}` with bodies |
| `get_message` | `messageId`* | one message with body |
| `get_attachment` | `messageId`*, `fileId`* | metadata (`filename`, `contentType`, `size`, `path`, `resourceUri`), plus inline `data` for text up to 200 KB or an image block for images up to 200 KB |
| `list_scheduled_sends` | `accountId` | drafts with a Send Later schedule: `state` `scheduled`, `due` or `failed` (with `error`) |
| `list_events` | `start`*, `end`*, `accountId`, `calendarIds` | `{count, events}`; each has `eventId`, `calendarId`, `accountId`, `title`, `start`/`end` (all-day: `startDate`/`endDate`, end exclusive), `location`, `isRecurring`, `organizer`, `attendees`, `myStatus`, `isCancelled` |
| `get_contact` | `contactId`*, `accountId` | name, emailAddresses, phoneNumbers, addresses, company, title, birthdays, urls, notes, `readonly` |
| `get_message_tracking` | `messageId`* | opens and link clicks for one sent, tracked message |
| `list_tracked_messages` | `accountId`, `sentAfter`, `sentBefore`, `openedAfter`, `trackingType` (`opens`/`links`/`either`), `minOpens`, `minClicks`, `limit` (50), `offset` | `{total, messages}` |
| `get_top_links` | `accountId`, `sentAfter`, `sentBefore`, `limit` (10) | tracked links by clicks |

A thread summary has `id`, `subject`, `snippet`, `unread`, `starred`,
`lastMessageReceivedTimestamp`, `participantCount`, `attachmentCount`, `accountId` and
`categories` (`[{id, name}]`, its folders and labels).

A message has `id`, `headerMessageId`, `threadId`, `from`, `to`, `cc`, `bcc` (each
`[{name, email}]`), `subject`, `date`, `body` (HTML), `snippet`, `files`
(`[{id, filename, contentType, size, isInline}]`), `unread`, `starred` and `draft`.
`draft: true` is the user's own unsent draft.

Attachments are also readable as the resource `attachment://{messageId}/{fileId}`.

## Read and draft (`read-write`)

| Tool | Arguments | Notes |
| --- | --- | --- |
| `create_draft` | `to`*, `subject`*, `body`* (HTML), `cc`, `bcc`, `accountId`, `attachments` | recipients are `[{name?, email}]`; `attachments` are absolute paths of files on this computer; returns `{id, headerMessageId, accountId, threadId, subject, draft: true}`, plus `attachments` (`[{id, filename, size}]`) when it has any |
| `reply_to_thread` | `threadId`*, `body`* (HTML), `type` (`reply`/`reply-all`), `messageId`, `attachments` | replies to the newest non-draft message (or `messageId`); also returns `repliedTo`, `quoted`, `to`, `cc`, `existingDrafts`, `note`, `attachments` |
| `update_draft` | `id` or `headerMessageId`, `subject`, `body`, `to`, `cc`, `bcc`, `attachments`, `removeAttachments`, `accountId` | fields given replace the old ones; `body` replaces the quote too; `attachments` adds files by absolute path (a file already attached isn't added twice), `removeAttachments` takes file ids or filenames from the draft's `files`; refused while the draft is being sent or scheduled |
| `delete_draft` | `id` or `headerMessageId`, `accountId` | refused while the draft is being sent; a pending scheduled send is cancelled with it on current engines |
| `archive_threads` | `threadIds`* | Gmail removes the Inbox label; other accounts move to Archive (or All Mail) |
| `trash_threads` | `threadIds`* | |
| `move_to_folder` | `threadIds`*, `folderId`* | |
| `set_labels` | `threadIds`*, `labelsToAdd`, `labelsToRemove` | label ids; Gmail only |
| `set_unread` | `threadIds`*, `unread`* | |
| `set_starred` | `threadIds`*, `starred`* | |
| `snooze_threads` | `threadIds`*, `until`* | moves to Snoozed; they come back to the inbox at `until` |
| `unsnooze_threads` | `threadIds`* | back to the inbox now |
| `cancel_scheduled_send` | `headerMessageId`* | error when the draft has no schedule |
| `create_event` | `calendarId`*, `title`*, `start`*, `end`*, `allDay`, `location`, `description`, `recurrenceRule`, `accountId`, `attendees` | with `attendees` it needs `read-write-send` |
| `update_event` | `eventId`*, `title`, `start`, `end`, `location`, `description`, `attendees`, `delete`, `scope` (`this`/`following`/`all`), `occurrenceStart`, `accountId` | needs `read-write-send` with `attendees`, or for a meeting the account organizes that has guests |
| `create_contact` | `name`, `email`, `phone`, `company`, `title`, `notes`, `accountId` | at least a name or an email |
| `update_contact` | `contactId`*, same fields | `email`/`phone` replace the whole list with one value |

## Read, draft and send (`read-write-send`)

| Tool | Arguments | Notes |
| --- | --- | --- |
| `send_draft` | `headerMessageId`* | sends now; the From address must be the account's |
| `schedule_send` | `headerMessageId`*, `sendAt`*, `actionKey` | `actionKey` defaults to `send`; read the result's `note` |
| `unsubscribe` | `messageId`*, `method` (`one-click`/`mailto`/`web`) | needs a List-Unsubscribe header; a web-only list returns a `url` for the user |
| `respond_to_event` | `eventId`*, `status`* (`accepted`/`tentative`/`declined`), `comment`, `occurrenceStart`, `proposedStart` + `proposedEnd`, `accountId` | mails the organizer; with a proposal it only proposes |
