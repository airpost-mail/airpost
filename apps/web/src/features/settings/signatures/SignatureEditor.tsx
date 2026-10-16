import { Trash2 } from 'lucide-react';
import { useId } from 'react';
import { EmailFrame } from '@/components/email';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { useAccounts } from '@/engine';
import {
  removeSignature,
  setDefaultSignature,
  updateSignature,
  useSignatures,
  type Signature,
} from './store';

export function SignatureEditor({ signature }: { signature: Signature }) {
  const uid = useId();
  const { defaults } = useSignatures();
  const { data: accounts = [] } = useAccounts();

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5">
          <Label htmlFor={`${uid}-title`} className="text-xs text-muted-foreground">
            Name
          </Label>
          <Input
            id={`${uid}-title`}
            value={signature.title}
            onChange={(e) => updateSignature(signature.id, { title: e.target.value })}
          />
        </div>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Delete signature"
          className="text-muted-foreground hover:text-destructive"
          onClick={() => removeSignature(signature.id)}
        >
          <Trash2 />
        </Button>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${uid}-body`} className="text-xs text-muted-foreground">
          HTML
        </Label>
        <Textarea
          id={`${uid}-body`}
          rows={6}
          spellCheck={false}
          className="font-mono text-xs"
          placeholder="<p>Maya Okafor<br>Okafor Studio</p>"
          value={signature.body}
          onChange={(e) => updateSignature(signature.id, { body: e.target.value })}
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs text-muted-foreground">Preview</span>
        <div className="rounded-lg border bg-reader p-2">
          <EmailFrame
            html={signature.body || '<p style="color:#888">Nothing yet.</p>'}
            title="Signature preview"
            className="rounded-md"
          />
        </div>
      </div>

      {accounts.length > 0 && (
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-2 text-xs text-muted-foreground">Use by default for</legend>
          {accounts.map((a) => {
            const id = `${uid}-acct-${a.id}`;
            return (
              <div key={a.id} className="flex items-center gap-2.5">
                <Checkbox
                  id={id}
                  checked={defaults[a.emailAddress] === signature.id}
                  onCheckedChange={(on) =>
                    setDefaultSignature(a.emailAddress, on === true ? signature.id : null)
                  }
                />
                <Label htmlFor={id} className="flex items-center gap-2 text-sm font-normal">
                  <span
                    aria-hidden
                    className="size-1.5 rounded-full"
                    style={{ backgroundColor: a.color }}
                  />
                  {a.emailAddress}
                </Label>
              </div>
            );
          })}
        </fieldset>
      )}
    </div>
  );
}
