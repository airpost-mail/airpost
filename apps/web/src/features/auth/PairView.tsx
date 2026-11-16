import { useNavigate } from '@tanstack/react-router';
import { CircleAlert, LoaderCircle } from 'lucide-react';
import { useEffect, useState, type FormEvent } from 'react';
import { Wordmark } from '@/components/Logo';
import { ThemeToggle } from '@/components/ThemeToggle';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { codeFromHash, defaultDeviceName, formatPairingCode } from './pairing-code';
import { gatewayTransport, getSignOutNotice, pair } from './session';

export function PairView({ redirectTo }: { redirectTo?: string }) {
  const navigate = useNavigate();
  const mock = gatewayTransport.kind === 'mock';
  // A `/pair#<code>` link: read the code, then take it out of the address bar and history
  // (the effect below). A link opened in a tab already on /pair only changes the fragment.
  const [linked, setLinked] = useState(() => codeFromHash(window.location.hash));
  const fromLink = linked !== null;
  const [code, setCode] = useState(linked ? formatPairingCode(linked) : '');
  const [deviceName, setDeviceName] = useState(defaultDeviceName);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [notice] = useState(getSignOutNotice);

  useEffect(() => {
    const strip = () => {
      if (!window.location.hash) return;
      history.replaceState(history.state, '', window.location.pathname + window.location.search);
    };
    strip();
    const onHashChange = () => {
      const next = codeFromHash(window.location.hash);
      strip();
      if (!next) return;
      setLinked(next);
      setCode(formatPairingCode(next));
      setError(null);
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await pair(code, deviceName);
      await navigate({ href: redirectTo ?? '/', replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Pairing failed.');
      setSubmitting(false);
    }
  };

  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
      <header className="relative flex h-11 shrink-0 items-center justify-between px-4">
        <Wordmark />
        <ThemeToggle />
        <span className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent" />
      </header>

      <main className="flex flex-1 justify-center px-6 pt-[12vh] pb-16">
        <form
          onSubmit={submit}
          noValidate
          aria-labelledby="pair-title"
          className="flex w-full max-w-[22rem] animate-in flex-col duration-(--duration-slow) ease-(--ease-out) fade-in slide-in-from-bottom-1"
        >
          <p className="mb-2 text-2xs tracking-widest text-muted-foreground uppercase">
            {window.location.host}
          </p>
          <h1 id="pair-title" className="text-display font-semibold tracking-tight">
            Pair this browser
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {fromLink
              ? 'Check the name, then pair.'
              : notice === 'revoked'
                ? 'This browser was unpaired. Pair it again to carry on.'
                : notice === 'user'
                  ? 'Signed out.'
                  : 'Run airpost-gateway pair where your engine runs, then enter the code it prints.'}
          </p>

          <div className="mt-6 flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pair-code">Pairing code</Label>
              <Input
                id="pair-code"
                autoComplete="one-time-code"
                autoCapitalize="characters"
                autoFocus={!fromLink}
                spellCheck={false}
                placeholder="XXXX-XXXX-XXXX"
                value={code}
                onChange={(e) => {
                  setCode(e.target.value);
                  setError(null);
                }}
                className="font-mono tracking-wider tabular"
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pair-device">Name for this browser</Label>
              <Input
                id="pair-device"
                autoComplete="off"
                maxLength={64}
                value={deviceName}
                onChange={(e) => setDeviceName(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Shown by{' '}
                <code className="font-mono text-2xs text-foreground">
                  airpost-gateway devices list
                </code>
                .
              </p>
            </div>
          </div>

          {error && (
            <Alert variant="destructive" className="mt-5 animate-in fade-in">
              <CircleAlert />
              <AlertDescription className="text-destructive">{error}</AlertDescription>
            </Alert>
          )}

          <Button
            type="submit"
            size="lg"
            className="mt-6"
            disabled={submitting || !code.trim()}
            autoFocus={fromLink}
          >
            {submitting && <LoaderCircle className="animate-spin" aria-hidden />}
            {submitting ? 'Pairing…' : 'Pair'}
          </Button>

          {mock && (
            <p className="mt-4 text-xs text-muted-foreground">
              No gateway is configured, so Airpost runs a demo engine in this tab. Any 12-character
              code works, for example ABCD-EFGH-JKLM.
            </p>
          )}
        </form>
      </main>
    </div>
  );
}
