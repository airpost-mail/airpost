import React from 'react';
import {
  localized,
  Actions,
  Thread,
  TaskFactory,
  FocusedPerspectiveStore,
} from 'mailspring-exports';
import { Button } from '../../components/kit/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/kit/tooltip';
import { ownKeys } from '../../components/kit/keys';

// Space and Enter click the row's buttons and don't also reach the keymap.
const ownClickKeys = ownKeys([' ', 'Enter']);

/* A hover action on a thread row. The icon is the `action-<name>` class's background image, which
   themes and plugins (thread-snooze's `action-snooze`) style. */
function ThreadQuickActionButton({
  name,
  label,
  order,
  onClick,
}: {
  name: string;
  label: string;
  order?: number;
  onClick: (event: React.MouseEvent) => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* The trigger's own data-slot would replace the button's. */}
        <Button
          data-slot="button"
          variant="outline"
          aria-label={label}
          style={{ order }}
          className={`action action-${name}`}
          onClick={onClick}
          onKeyDown={ownClickKeys}
        />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

const sameThreadId = (prev: { thread: Thread }, next: { thread: Thread }) =>
  prev.thread.id === next.thread.id;

export const ThreadArchiveQuickAction: React.FC<{ thread: Thread }> = React.memo(({ thread }) => {
  const allowed = FocusedPerspectiveStore.current().canArchiveThreads([thread]);
  if (!allowed) {
    return <span />;
  }

  const onArchive = (event: React.MouseEvent) => {
    const tasks = TaskFactory.tasksForArchiving({
      source: 'Quick Actions: Thread List',
      threads: [thread],
      perspective: FocusedPerspectiveStore.current(),
    });
    Actions.queueTasks(tasks);
    event.stopPropagation();
  };

  return (
    <ThreadQuickActionButton
      name="archive"
      label={localized('Archive')}
      order={100}
      onClick={onArchive}
    />
  );
}, sameThreadId);
ThreadArchiveQuickAction.displayName = 'ThreadArchiveQuickAction';

export const ThreadTrashQuickAction: React.FC<{ thread: Thread }> = React.memo(({ thread }) => {
  const allowed = FocusedPerspectiveStore.current().canMoveThreadsTo([thread], 'trash');
  if (!allowed) {
    return <span />;
  }

  const onRemove = (event: React.MouseEvent) => {
    const tasks = TaskFactory.tasksForMovingToTrash({
      source: 'Quick Actions: Thread List',
      threads: [thread],
    });
    Actions.queueTasks(tasks);
    event.stopPropagation();
  };

  return (
    <ThreadQuickActionButton
      name="trash"
      label={localized('Trash')}
      order={110}
      onClick={onRemove}
    />
  );
}, sameThreadId);
ThreadTrashQuickAction.displayName = 'ThreadTrashQuickAction';
