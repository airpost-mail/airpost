import { CircleAlert, CircleCheck, LoaderCircle, Lock } from 'lucide-react';
import { useId, useState, type FormEvent } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Status } from '../components/StatusDot';
import type { FieldKey, ProviderDef } from './providers';
import {
  clearSecret,
  isProviderConfigured,
  saveProvider,
  setProviderEnabled,
  useIntegrations,
} from './store';
import { useIntegrationTest } from './useIntegrationTest';

type Draft = Partial<Record<FieldKey, string>>;

export function ProviderCard({ def }: { def: ProviderDef }) {
  const state = useIntegrations();
  const config = state.providers[def.id];
  const heldSecrets = state.secrets[def.id];
  const configured = isProviderConfigured(state, def.id);
  const [draft, setDraft] = useState<Draft>(() => ({ ...config.values }));
  const test = useIntegrationTest(def.id);
  const uid = useId();

  const dirty = def.fields.some((f) =>
    f.secret ? !!draft[f.key]?.trim() : (draft[f.key] ?? '') !== (config.values[f.key] ?? ''),
  );

  const onSave = (e: FormEvent) => {
    e.preventDefault();
    saveProvider(def.id, draft);
    // Secret inputs are cleared once handed over; the field then shows "saved".
    setDraft((d) => {
      const next = { ...d };
      for (const f of def.fields) if (f.secret) delete next[f.key];
      return next;
    });
    test.reset();
    toast(`${def.name} settings saved`);
  };

  return (
    <form
      onSubmit={onSave}
      className="flex flex-col gap-4 px-4 py-4"
      aria-label={`${def.name} settings`}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-0.5">
          <div className="flex items-center gap-3">
            <Label htmlFor={`${uid}-enabled`} className="text-sm font-semibold">
              {def.name}
            </Label>
            <Status tone={!config.enabled ? 'muted' : configured ? 'success' : 'warning'}>
              {!config.enabled ? 'Off' : configured ? 'On' : 'Needs setup'}
            </Status>
          </div>
          <p className="text-xs text-muted-foreground">{def.blurb}</p>
        </div>
        <Switch
          id={`${uid}-enabled`}
          checked={config.enabled}
          onCheckedChange={(on) => setProviderEnabled(def.id, on)}
        />
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        {def.fields.map((f) => {
          const held = f.secret && heldSecrets.includes(f.key);
          return (
            <div key={f.key} className={f.key === 'url' ? 'sm:col-span-2' : undefined}>
              <div className="mb-1.5 flex items-center justify-between">
                <Label htmlFor={`${uid}-${f.key}`} className="text-xs text-muted-foreground">
                  {f.label}
                  {f.required && <span className="sr-only"> (required)</span>}
                </Label>
                {held && (
                  <button
                    type="button"
                    className="text-2xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                    onClick={() => clearSecret(def.id, f.key)}
                  >
                    Clear
                  </button>
                )}
              </div>
              <Input
                id={`${uid}-${f.key}`}
                type={f.secret ? 'password' : f.key === 'url' ? 'url' : 'text'}
                autoComplete="off"
                spellCheck={false}
                className={
                  f.secret || f.key === 'url'
                    ? 'font-mono text-xs placeholder:font-sans'
                    : undefined
                }
                placeholder={held ? '•••••••• saved for this session' : f.placeholder}
                value={draft[f.key] ?? ''}
                onChange={(e) => setDraft((d) => ({ ...d, [f.key]: e.target.value }))}
              />
            </div>
          );
        })}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Lock className="size-3.5 shrink-0" />
          {def.privacy}
        </p>
        <div className="flex items-center gap-2">
          <TestResult pending={test.isPending} outcome={test.data} />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={test.isPending}
            onClick={() => test.mutate(draft)}
          >
            Test
          </Button>
          <Button type="submit" size="sm" disabled={!dirty}>
            Save
          </Button>
        </div>
      </div>
    </form>
  );
}

function TestResult({
  pending,
  outcome,
}: {
  pending: boolean;
  outcome: ReturnType<typeof useIntegrationTest>['data'];
}) {
  if (pending) {
    return (
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground" role="status">
        <LoaderCircle className="size-3.5 animate-spin" /> Testing
      </span>
    );
  }
  if (!outcome) return null;
  return outcome.ok ? (
    <span className="flex items-center gap-1.5 text-xs text-success" role="status">
      <CircleCheck className="size-3.5" /> Connected to {outcome.host}
    </span>
  ) : (
    <span
      className="flex max-w-64 items-center gap-1.5 text-xs text-destructive"
      role="status"
      title={outcome.error}
    >
      <CircleAlert className="size-3.5 shrink-0" />
      <span className="truncate">{outcome.error}</span>
    </span>
  );
}
