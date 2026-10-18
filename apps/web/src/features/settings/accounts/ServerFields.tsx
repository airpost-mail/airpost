import { Checkbox } from '@/components/ui/checkbox';
import { Label } from '@/components/ui/label';
import type { SecurityMode } from '@/engine';
import { ChoiceSelect, type Choice } from '../components/ChoiceSelect';
import { asksCleartextConsent, type AccountForm, type FormErrors } from './account-draft';
import { TextField } from './TextField';

const SECURITY: Choice<SecurityMode>[] = [
  { value: 'SSL / TLS', label: 'SSL / TLS' },
  { value: 'STARTTLS', label: 'STARTTLS' },
  { value: 'none', label: 'None' },
];

type Proto = 'imap' | 'smtp';

/** Host, port, security and username for one protocol. */
export function ServerFields({
  proto,
  form,
  errors,
  onChange,
}: {
  proto: Proto;
  form: AccountForm;
  errors: FormErrors;
  onChange: (patch: Partial<AccountForm>) => void;
}) {
  const host = `${proto}Host` as const;
  const port = `${proto}Port` as const;
  const security = `${proto}Security` as const;
  const username = `${proto}Username` as const;
  const allowCleartext = `${proto}AllowCleartext` as const;
  const consentError = errors[allowCleartext];
  const label = proto.toUpperCase();
  const set = <K extends keyof AccountForm>(k: K, v: AccountForm[K]) =>
    onChange({ [k]: v } as Partial<AccountForm>);

  return (
    <fieldset className="grid grid-cols-[1fr_6rem_9rem] gap-3">
      <legend className="mb-2 text-2xs font-medium tracking-widest text-muted-foreground uppercase">
        {label}
      </legend>
      <TextField
        id={`acct-${host}`}
        label="Server"
        mono
        value={form[host]}
        error={errors[host]}
        onChange={(v) => set(host, v)}
      />
      <TextField
        id={`acct-${port}`}
        label="Port"
        mono
        inputMode="numeric"
        value={form[port]}
        error={errors[port]}
        onChange={(v) => set(port, v)}
      />
      <div className="flex flex-col gap-1.5">
        <label htmlFor={`acct-${security}`} className="text-xs text-muted-foreground">
          Security
        </label>
        <ChoiceSelect
          id={`acct-${security}`}
          className="w-full"
          value={form[security]}
          choices={SECURITY}
          onChange={(v) => set(security, v)}
        />
      </div>
      {asksCleartextConsent(form, proto) && (
        <div className="col-span-3 flex flex-col gap-1">
          <div className="flex items-center gap-2.5">
            <Checkbox
              id={`acct-${allowCleartext}`}
              checked={form[allowCleartext]}
              aria-invalid={!!consentError || undefined}
              aria-describedby={consentError ? `acct-${allowCleartext}-error` : undefined}
              onCheckedChange={(on) => set(allowCleartext, on === true)}
            />
            <Label htmlFor={`acct-${allowCleartext}`} className="text-sm font-normal">
              Send password unencrypted
            </Label>
          </div>
          {consentError && (
            <p id={`acct-${allowCleartext}-error`} className="text-xs text-destructive">
              {consentError}
            </p>
          )}
        </div>
      )}
      <TextField
        id={`acct-${username}`}
        className="col-span-3"
        label="Username"
        value={form[username]}
        onChange={(v) => set(username, v)}
      />
    </fieldset>
  );
}
