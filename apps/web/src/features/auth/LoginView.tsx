import { useNavigate } from '@tanstack/react-router';
import { CircleAlert, LoaderCircle } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import { Wordmark } from '@/components/Logo';
import { ThemeToggle } from '@/components/ThemeToggle';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { defaultServerUrl } from './server-url';
import { ServerField } from './ServerField';
import { authTransport, getSignOutNotice, signIn, type Credentials } from './session';
import { useServerProbe } from './use-server-probe';

type Method = Credentials['method'];

export function LoginView({ redirectTo }: { redirectTo?: string }) {
  const navigate = useNavigate();
  const mock = authTransport.kind === 'mock';
  const [serverUrl, setServerUrl] = useState(defaultServerUrl);
  const [method, setMethod] = useState<Method>('password');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [notice] = useState(getSignOutNotice);
  const probe = useServerProbe(serverUrl);
  const serverOk = probe.isSuccess;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (submitting || !serverOk) return;
    setSubmitting(true);
    setError(null);
    try {
      await signIn(
        method === 'password'
          ? { method, serverUrl, email, password }
          : { method, serverUrl, token },
      );
      // The password lives only in component state; drop it as soon as it has been used.
      setPassword('');
      await navigate({ href: redirectTo ?? '/', replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Sign-in failed.');
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
          aria-labelledby="login-title"
          className="flex w-full max-w-[22rem] animate-in flex-col duration-(--duration-slow) ease-(--ease-out) fade-in slide-in-from-bottom-1"
        >
          <p className="mb-2 text-2xs tracking-widest text-muted-foreground uppercase">
            Tern server
          </p>
          <ServerField
            url={serverUrl}
            onChange={(url) => {
              setServerUrl(url);
              setError(null);
            }}
            probe={probe}
            mock={mock}
          />

          <h1 id="login-title" className="mt-10 text-display font-semibold tracking-tight">
            Sign in
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {notice === 'revoked'
              ? 'The server signed you out. Sign in again to carry on.'
              : notice === 'user'
                ? 'Signed out.'
                : 'Connect to your Tern server.'}
          </p>

          <Tabs
            value={method}
            onValueChange={(v) => {
              setMethod(v as Method);
              setError(null);
            }}
            className="mt-6 gap-4"
          >
            <TabsList variant="line" className="h-8 justify-start border-b border-hairline px-0">
              <TabsTrigger value="password" className="flex-none px-2">
                Password
              </TabsTrigger>
              <TabsTrigger value="device-token" className="flex-none px-2">
                Device token
              </TabsTrigger>
            </TabsList>

            <TabsContent value="password" className="flex flex-col gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="login-email">Email</Label>
                <Input
                  id="login-email"
                  type="email"
                  autoComplete="username"
                  autoFocus
                  spellCheck={false}
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="login-password">Password</Label>
                <Input
                  id="login-password"
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </div>
            </TabsContent>

            <TabsContent value="device-token" className="flex flex-col gap-1.5">
              <Label htmlFor="login-token">Device token</Label>
              <Input
                id="login-token"
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder="tern_…"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                className="font-mono text-xs"
              />
              <p className="text-xs text-muted-foreground">
                Create one on the server with{' '}
                <code className="font-mono text-2xs text-foreground">
                  tern-backend token create
                </code>
                .
              </p>
            </TabsContent>
          </Tabs>

          {error && (
            <Alert variant="destructive" className="mt-5 animate-in fade-in">
              <CircleAlert />
              <AlertDescription className="text-destructive">{error}</AlertDescription>
            </Alert>
          )}

          <Button type="submit" size="lg" className="mt-6" disabled={submitting || !serverOk}>
            {submitting && <LoaderCircle className="animate-spin" aria-hidden />}
            {submitting ? 'Signing in…' : 'Sign in'}
          </Button>

          {mock && (
            <p className="mt-4 text-xs text-muted-foreground">
              No engine is configured, so Tern runs a demo engine in this tab. Any email and
              password work.
            </p>
          )}
        </form>
      </main>
    </div>
  );
}
