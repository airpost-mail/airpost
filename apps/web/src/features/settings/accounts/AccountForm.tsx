import { ChevronDown, CircleAlert, LoaderCircle } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import { toast } from 'sonner';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { useAccounts } from '@/engine';
import { cn } from '@/lib/utils';
import { engineErrorMessage } from '../engine-call';
import { AccountTestError, useAddAccount } from './actions';
import {
  ACCOUNT_COLORS,
  buildAccount,
  formForPreset,
  validateForm,
  type AccountForm as Form,
  type FormErrors,
} from './account-draft';
import type { ProviderPreset } from './presets';
import { ServerFields } from './ServerFields';
import { TextField } from './TextField';

const SERVER_KEYS: (keyof Form)[] = [
  'imapHost',
  'imapPort',
  'imapSecurity',
  'imapUsername',
  'smtpHost',
  'smtpPort',
  'smtpSecurity',
  'smtpUsername',
];

export function AccountForm({ preset, onAdded }: { preset: ProviderPreset; onAdded: () => void }) {
  const { data: accounts = [] } = useAccounts();
  const add = useAddAccount();
  const [form, setForm] = useState<Form>(() => formForPreset(preset));
  // Server fields the user typed in; the rest follow the email address.
  const [touched, setTouched] = useState<Set<keyof Form>>(() => new Set());
  const [errors, setErrors] = useState<FormErrors>({});
  const [advanced, setAdvanced] = useState(preset.id === 'imap');
  const [failure, setFailure] = useState<{ message: string; log?: string } | null>(null);

  const change = (patch: Partial<Form>) => {
    const keys = Object.keys(patch) as (keyof Form)[];
    const nextTouched = new Set(touched);
    for (const k of keys) if (SERVER_KEYS.includes(k)) nextTouched.add(k);
    setTouched(nextTouched);
    setForm((prev) => {
      let next = { ...prev, ...patch };
      if ('email' in patch) {
        const derived = formForPreset(preset, next);
        for (const k of SERVER_KEYS) {
          if (!nextTouched.has(k)) next = { ...next, [k]: derived[k] };
        }
      }
      return next;
    });
    setErrors((e) => {
      const n = { ...e };
      for (const k of keys) delete n[k];
      return n;
    });
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setFailure(null);
    const errs = validateForm(form);
    setErrors(errs);
    if (Object.keys(errs).some((k) => SERVER_KEYS.includes(k as keyof Form))) setAdvanced(true);
    if (Object.keys(errs).length) return;

    const account = await buildAccount(form, {
      provider: preset.provider,
      color: ACCOUNT_COLORS[accounts.length % ACCOUNT_COLORS.length],
    });
    if (accounts.some((a) => a.id === account.id)) {
      setFailure({ message: 'This account is already in Tern.' });
      return;
    }
    add.mutate(account, {
      onSuccess: () => {
        toast(`Added ${account.emailAddress}`);
        onAdded();
      },
      onError: (err) =>
        setFailure({
          message: engineErrorMessage(err),
          log: err instanceof AccountTestError ? err.log : undefined,
        }),
    });
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-6" aria-label="Account">
      <div className="grid grid-cols-2 gap-4">
        <TextField
          id="acct-name"
          label="Your name"
          autoComplete="name"
          value={form.name}
          onChange={(name) => change({ name })}
        />
        <TextField
          id="acct-email"
          label="Email address"
          type="email"
          autoComplete="email"
          autoFocus
          value={form.email}
          error={errors.email}
          onChange={(email) => change({ email })}
        />
        <TextField
          id="acct-password"
          className="col-span-2"
          label="Password"
          type="password"
          autoComplete="current-password"
          value={form.password}
          error={errors.password}
          hint={preset.passwordHint}
          onChange={(password) => change({ password })}
        />
      </div>

      <section className="rounded-lg border bg-card">
        <button
          type="button"
          aria-expanded={advanced}
          aria-controls="acct-servers"
          onClick={() => setAdvanced((a) => !a)}
          className="flex w-full items-center justify-between px-4 py-3 text-left text-sm font-medium"
        >
          <span className="flex flex-col">
            Server settings
            {!advanced && (
              <span className="font-mono text-2xs font-normal text-muted-foreground">
                {form.imapHost || 'imap'} · {form.smtpHost || 'smtp'}
              </span>
            )}
          </span>
          <ChevronDown
            className={cn(
              'size-4 text-muted-foreground transition-transform duration-(--duration-base)',
              advanced && 'rotate-180',
            )}
          />
        </button>
        {advanced && (
          <div id="acct-servers" className="flex flex-col gap-5 border-t border-hairline px-4 py-4">
            <ServerFields proto="imap" form={form} errors={errors} onChange={change} />
            <ServerFields proto="smtp" form={form} errors={errors} onChange={change} />
            <TextField
              id="acct-smtpPassword"
              label="SMTP password"
              type="password"
              autoComplete="off"
              placeholder="Same as above"
              value={form.smtpPassword}
              onChange={(smtpPassword) => change({ smtpPassword })}
            />
            <div className="flex items-center justify-between gap-4">
              <Label htmlFor="acct-insecure" className="flex flex-col items-start gap-0.5">
                <span className="text-sm font-medium">Allow insecure certificates</span>
                <span className="text-xs font-normal text-muted-foreground">
                  Only for servers with a self-signed certificate you trust.
                </span>
              </Label>
              <Switch
                id="acct-insecure"
                checked={form.allowInsecureSsl}
                onCheckedChange={(allowInsecureSsl) => change({ allowInsecureSsl })}
              />
            </div>
          </div>
        )}
      </section>

      {failure && (
        <Alert variant="destructive">
          <CircleAlert />
          <AlertTitle>Couldn’t add the account</AlertTitle>
          <AlertDescription>
            <p>{failure.message}</p>
            {failure.log && (
              <details className="mt-2">
                <summary className="cursor-pointer text-xs">Connection log</summary>
                <pre className="mt-1 max-h-48 overflow-auto font-mono text-2xs whitespace-pre-wrap">
                  {failure.log}
                </pre>
              </details>
            )}
          </AlertDescription>
        </Alert>
      )}

      <div className="flex items-center justify-end gap-3">
        {add.isPending && (
          <span role="status" className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <LoaderCircle className="size-3.5 animate-spin" /> Checking IMAP and SMTP
          </span>
        )}
        <Button type="submit" disabled={add.isPending}>
          Add account
        </Button>
      </div>
    </form>
  );
}
