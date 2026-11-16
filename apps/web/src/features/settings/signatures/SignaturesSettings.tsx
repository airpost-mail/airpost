import { Plus } from 'lucide-react';
import { useState } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { SettingsPage } from '../components/SettingsPage';
import { SignatureEditor } from './SignatureEditor';
import { createSignature, useSignatures } from './store';

export function SignaturesSettings() {
  const { signatures } = useSignatures();
  const list = Object.values(signatures);
  const [picked, setPicked] = useState<string | null>(null);
  const selected = (picked && signatures[picked]) || list[0] || null;

  const add = () => setPicked(createSignature(`Signature ${list.length + 1}`).id);

  return (
    <SettingsPage
      title="Signatures"
      description="Added to new messages from the accounts you choose."
      actions={
        list.length > 0 && (
          <Button size="sm" variant="outline" onClick={add}>
            <Plus /> New signature
          </Button>
        )
      }
    >
      {!selected ? (
        <EmptyState
          className="h-auto py-16"
          title="No signatures yet"
          description="Write one once and Airpost adds it to every new message."
          action={
            <Button size="sm" onClick={add}>
              <Plus /> New signature
            </Button>
          }
        />
      ) : (
        <div className="grid grid-cols-[11rem_1fr] gap-6">
          <ul aria-label="Signatures" className="flex flex-col gap-px">
            {list.map((s) => (
              <li key={s.id}>
                <button
                  type="button"
                  aria-current={s.id === selected.id}
                  onClick={() => setPicked(s.id)}
                  className={cn(
                    'flex h-8 w-full items-center truncate rounded-md px-2 text-left text-sm text-muted-foreground transition-colors duration-(--duration-fast) hover:bg-row-hover hover:text-foreground',
                    s.id === selected.id && 'bg-row-selected text-foreground',
                  )}
                >
                  <span className="truncate">{s.title || 'Untitled'}</span>
                </button>
              </li>
            ))}
          </ul>
          <SignatureEditor key={selected.id} signature={selected} />
        </div>
      )}
    </SettingsPage>
  );
}
