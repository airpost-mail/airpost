import React from 'react';
import { localized, Actions, TaskFactory, ExtensionRegistry } from 'mailspring-exports';
import { Button } from '../../components/kit/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/kit/tooltip';
import { ownKeys } from '../../components/kit/keys';
import { ThreadWithMessagesMetadata } from './types';

// Space and Enter toggle the star and don't also reach the keymap.
const ownClickKeys = ownKeys([' ', 'Enter']);

function nonDraftMessages(thread: ThreadWithMessagesMetadata) {
  const msgs = thread.__messages;
  if (!msgs || !(msgs instanceof Array)) {
    return [];
  }
  return msgs.filter((m) => m.id && !m.draft);
}

function iconClassNames(thread: ThreadWithMessagesMetadata) {
  if (!thread) {
    return 'thread-icon-star-on-hover';
  }

  const extensionIconClassNames = ExtensionRegistry.ThreadList.extensions()
    .filter((ext) => ext.cssClassNamesForThreadListIcon != null)
    .reduce((prev, ext) => prev + ' ' + ext.cssClassNamesForThreadListIcon(thread), '')
    .trim();
  if (extensionIconClassNames.length > 0) {
    return extensionIconClassNames;
  }

  if (thread.starred) {
    return 'thread-icon-star';
  }
  if (thread.unread) {
    return 'thread-icon-unread thread-icon-star-on-hover';
  }

  const msgs = nonDraftMessages(thread);
  const last = msgs[msgs.length - 1];

  if (msgs.length > 1 && (last.from[0] != null ? last.from[0].isMe() : undefined)) {
    if (last.isForwarded()) {
      return 'thread-icon-forwarded thread-icon-star-on-hover';
    } else {
      return 'thread-icon-replied thread-icon-star-on-hover';
    }
  }

  return 'thread-icon-none thread-icon-star-on-hover';
}

const ThreadListIcon: React.FC<{ thread: ThreadWithMessagesMetadata }> = React.memo(
  ({ thread }) => {
    const starred = thread && thread.starred;
    const ariaLabel = starred ? localized('Unstar') : localized('Star');

    const onToggleStar = (event: React.MouseEvent) => {
      Actions.queueTask(
        TaskFactory.taskForInvertingStarred({
          threads: [thread],
          source: 'Thread List Icon',
        })
      );
      event.stopPropagation();
    };

    // The icon is the thread-icon-* class's background image (extensions add their own classes
    // through cssClassNamesForThreadListIcon). The link variant draws no box and no hover
    // background, so those images stay.
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {/* The trigger's own data-slot would replace the button's. */}
          <Button
            data-slot="button"
            variant="link"
            className={`thread-icon ${iconClassNames(thread)}`}
            tabIndex={-1}
            aria-label={ariaLabel}
            aria-pressed={starred || false}
            onClick={onToggleStar}
            onKeyDown={ownClickKeys}
          />
        </TooltipTrigger>
        <TooltipContent>{ariaLabel}</TooltipContent>
      </Tooltip>
    );
  },
  (prev, next) => prev.thread === next.thread
);
ThreadListIcon.displayName = 'ThreadListIcon';

export default ThreadListIcon;
