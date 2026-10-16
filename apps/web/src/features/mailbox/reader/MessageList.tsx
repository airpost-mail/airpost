import type { MessageJSON } from '@/engine';
import { foldRuns } from './expansion';
import { MessageItem } from './MessageItem';

export function MessageList({
  messages,
  own,
  isExpanded,
  onToggle,
  onExpand,
}: {
  messages: MessageJSON[];
  own: ReadonlySet<string>;
  isExpanded: (m: MessageJSON) => boolean;
  onToggle: (m: MessageJSON) => void;
  onExpand: (ms: MessageJSON[]) => void;
}) {
  return (
    <div className="flex flex-col">
      {foldRuns(messages, isExpanded).map((run) =>
        run.kind === 'message' ? (
          <MessageItem
            key={run.message.id}
            message={run.message}
            own={own}
            expanded={isExpanded(run.message)}
            onToggle={() => onToggle(run.message)}
          />
        ) : (
          <button
            key={`fold-${run.messages[0].id}`}
            type="button"
            onClick={() => onExpand(run.messages)}
            className="relative flex h-8 items-center border-b border-hairline px-6 text-left"
          >
            <span className="absolute inset-x-6 top-1/2 border-t border-dashed" aria-hidden />
            <span className="relative ml-10 bg-reader pr-2 font-mono text-2xs text-muted-foreground tabular hover:text-foreground">
              {run.messages.length} earlier messages
            </span>
          </button>
        ),
      )}
    </div>
  );
}
