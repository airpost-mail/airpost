# Undo and redo

How the desktop app undoes changes, and what a task needs for undo.

## Overview

The engine keeps the undo log (`undo.*`, `crates/tern-ipc/src/undo.rs`): one entry per change,
with the inverse built in Rust (`crates/tern-tasks/src/undo.rs`) from data the local phase
writes (`undoPlacements`, the flags and labels each message had, metadata `undoValue`,
`SyncbackEventTask` `undoData`). Entries belong to the client that made them, `tern-desktop`
for every window, so Cmd-Z never takes back what `tern` or an agent did (`tern undo --any`
can).

`UndoRedoStore` (`src/data/stores/undo-redo.ts`) is a view over the app's own entries: it reads
`undo.list` after each hello, follows `undo.changed`, shows the toast for a new entry, and
`undo()` / `redo()` send the entry's token to `undo.apply` / `undo.redo`.

## How It Works

### What makes an entry

- **Intents.** TaskFactory, `MailboxPerspective.tasksForRemovingItems` and drops, the
  pickers, the sidebar and the label and important icons queue an `IntentTask`
  (`src/data/tasks/intent.ts`): `Actions.queueTask(s)` as before, but EngineBridge calls the
  intent (`threads.archive`, `threads.move`, `categories.markRead`, ...) and the engine builds
  the tasks and the entry. `IntentTask.forThreads(intent, threads, params, source,
  description)` takes the toast's wording when the entry can't say it (the folder a move went
  to); `UndoRedoStore.describe(token, text)` attaches it.
- **Snooze** is `threads.snooze` (the thread-snooze plugin), **Undo Send** is the entry
  `draft.send` keeps for a send inside its undo window: DraftStore queues the undo-only
  `SyncbackMetadataTask` with `value.undoToken`, which the toast reads the countdown from.
- **Tasks queued with `canBeUndone`.** When every task of an `Actions.queueTask(s)` call has
  it and is of a class the engine can undo (ChangeUnread/Starred/Folder/Labels,
  SyncbackMetadata, SyncbackEvent, the contact group tasks), the store hands them to
  `undo.record` once their local phases ran, as one entry.

Marking a thread read as it opens (`TaskFactory.taskForSettingUnread({canBeUndone: false})`)
stays a plain `ChangeUnreadTask`, with no entry.

### What stays in the app

Two kinds of block never reach the log: an undo-only task without a token (a plugin's send
action: `draft.cancelSend`, or its `createUndoTasks()`), and tasks of a class the engine can't
undo (a v1 plugin's own subclass). Those are reversed as before: undo queues
`createUndoTasks()` of the versions the local phase left, redo queues
`createIdenticalTask()`. The patterns below are what such a class implements, and what the
engine's inverses follow.

## Patterns

### Pattern 1: Simple State Toggle (ChangeStarredTask)

For operations that toggle a boolean state:

```typescript
export class ChangeStarredTask extends ChangeMailTask {
  starred: boolean;

  // Inherited from ChangeMailTask: canBeUndone defaults to true

  createUndoTask() {
    const task = super.createUndoTask();
    task.starred = !this.starred;  // Simply flip the flag
    return task;
  }
}
```

### Pattern 2: State Snapshot (SyncbackMetadataTask, SyncbackEventTask)

For operations where the inverse isn't a simple toggle, snapshot the state on the task. See `SyncbackEventTask` (`src/data/tasks/syncback-event.ts`):

- `forUpdating({ event, undoData, description })` takes `undoData`, the state from *before* the edit, and snapshots the state being written as `newData`.
- The constructor sets `this.canBeUndone = !!this.undoData`, so a task built without `undoData` (creation, for example) is not undoable.
- `createUndoTask()` restores `undoData` and swaps the two snapshots, so undoing the undo reapplies the edit.
- `createIdenticalTask()` (redo) rebuilds the event from `newData` rather than reading `this.event`, which may have been mutated since the task was queued.
- Both re-send their snapshot through `matchEventSequence`, measured against the ICS this window last queued for the event. A snapshot carries the SEQUENCE it had when captured, and Google answers 409 to one below the SEQUENCE it holds, so a snapshot cannot be sent verbatim. A tie is accepted, and changed VEVENTs go out at one. Both first run `revertAddedOverrides`, because Google keeps an override a PUT leaves out: an occurrence overridden since the snapshot goes back as the snapshot's series has it.

`SyncbackMetadataTask` follows the same shape with `undoValue`.

### Pattern 3: Engine-Written Snapshot (ChangeFolderTask)

Sometimes the client cannot know the original state when it queues the task. A folder move
is the canonical case: a message may have copies in several folders, and only the sync
engine knows which copies it moved and from where. The engine writes that snapshot onto the
task's data during its local phase (the same mechanism `DestroyDraftTask` uses to receive
`stubIds`), and the task streams back to the client as a Task persist delta.

```typescript
export class ChangeFolderTask extends ChangeMailTask {
  folder: Folder;
  sourceFolderIds: string[];
  undoPlacements?: SourceFoldersByMessageId;     // written by the engine: where each moved copy came from
  restorePlacements?: SourceFoldersByMessageId;  // read by the engine on the undo task

  engineWritesUndoData = true;                   // tells UndoRedoStore to wait for the engine's version

  createUndoTasks() {
    const task = super.createUndoTask();         // isUndo = true
    task.folder = this._firstRestoreFolder();    // for the description only
    task.sourceFolderIds = [this.folder.id];     // the copies to send back are in the destination
    task.restorePlacements = this.undoPlacements;
    return [task];
  }
}
```

Three consequences for anyone using this pattern:

1. Set `engineWritesUndoData = true` on the task class. The engine's inverse reads the field
   from the stored task; a class reversed in the app (above) builds its undo from the version
   `TaskQueue.waitForPerformLocal()` resolves with, the **engine-updated** one.
2. Implement `createUndoTasks()` when the approximate undo can need several tasks (one per
   original folder), and return `[]` when nothing can be reversed; `createUndoTask()` stays
   for callers that can only queue one.
3. `createIdenticalTask()` (used for redo) must strip the engine-written field so a re-run
   starts with a clean snapshot.

## Implementation Steps

### Step 1: Add Undo Data Attributes

Add attributes to store the original state:

```typescript
static attributes = {
  ...Task.attributes,

  undoData: Attributes.Obj({
    modelKey: 'undoData',
  }),
  taskDescription: Attributes.String({
    modelKey: 'taskDescription',
  }),
};
```

### Step 2: Capture State Before Modification

Before modifying the model, capture its current state:

```typescript
function modifyEvent(event: Event, newData: EventData) {
  // Capture BEFORE modifying
  const undoData = {
    ics: event.ics,
    recurrenceStart: event.recurrenceStart,
    recurrenceEnd: event.recurrenceEnd,
  };

  // Now modify
  event.ics = newData.ics;
  event.recurrenceStart = newData.start;
  event.recurrenceEnd = newData.end;

  // Queue with undo support
  Actions.queueTask(SyncbackEventTask.forUpdating({
    event,
    undoData,
    description: localized('Edit event'),
  }));
}
```

### Step 3: Set canBeUndone and implement createUndoTask

```typescript
constructor(data: AttributeValues<typeof YourTask.attributes> = {}) {
  super(data);
  this.canBeUndone = !!this.undoData;
}

createUndoTask(): YourTask {
  if (!this.undoData) {
    throw new Error('Cannot create undo task without undoData');
  }

  // Create task that restores original state
  // Store current state in the undo task's undoData for redo
}
```

## Anti-Patterns to Avoid

### Don't Forget to Capture State First

❌ **Wrong**: Capturing after modification

```typescript
event.ics = newIcs;  // Modified first!
const undoData = { ics: event.ics };  // Too late - this is the new state
```

✅ **Correct**: Capture before modification

```typescript
const undoData = { ics: event.ics };  // Capture original
event.ics = newIcs;  // Now modify
```

## Testing Undo

1. Perform the action
2. Press Cmd/Ctrl+Z to undo
3. Verify the model is restored to original state
4. Press Cmd/Ctrl+Shift+Z to redo
5. Verify the model has the modified state again

## Related Files

- `src/data/stores/undo-redo.ts` - UndoRedoStore, over the engine's log
- `src/data/tasks/intent.ts` - IntentTask, the engine's intents queued as a task
- `crates/tern-tasks/src/undo.rs` - the engine's inverses
- `src/data/tasks/task.ts` - Base Task class
- `src/data/tasks/change-mail.ts` - Base class for mail changes
- `src/data/tasks/syncback-metadata.ts` - Example of snapshot pattern
- `src/data/tasks/syncback-event.ts` - Calendar event undo implementation
- `src/data/tasks/change-folder.ts` - Engine-written snapshot (`undoPlacements`)
