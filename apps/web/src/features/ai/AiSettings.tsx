import { useMutation, useQuery } from '@tanstack/react-query';
import { Button } from '@/components/ui/button';
import {
  EngineFeature,
  ensureConnected,
  useAccounts,
  useEngine,
  useEngineFeatures,
  type AiUsageRow,
} from '@/engine';
import { SettingsPage } from '@/features/settings/components/SettingsPage';
import { SettingRow, SettingsSection } from '@/features/settings/components/SettingsSection';
import { Status } from '@/features/settings/components/StatusDot';
import { errorMessage } from './api';
import {
  AI_FEATURES,
  KINDS,
  destinationNote,
  providerName,
  type AiFeature,
  type AiProvider,
} from './config';
import { aiKeys, unavailableMessage, useAiAvailability, useAiSettings } from './hooks';

const FEATURE_LABEL: Record<AiFeature, { title: string; description: string }> = {
  summary: { title: 'Summaries', description: 'Summarize in a thread.' },
  reply: { title: 'Reply suggestions', description: 'Suggest in a reply.' },
  triage: { title: 'Triage', description: 'Needs reply and Important on new inbox mail.' },
};

const PRIVACY: { feature: string; sent: string; when: string }[] = [
  {
    feature: 'Summary',
    sent: "The thread's subject, sender names, dates and plain-text bodies",
    when: 'You click Summarize',
  },
  {
    feature: 'Reply',
    sent: 'The same, plus what you ask for and your name',
    when: 'You ask for suggestions or a reply',
  },
  {
    feature: 'Triage',
    sent: "The latest message, and the other messages' subjects and senders",
    when: 'Each new inbox thread, in accounts opted in',
  },
];

/**
 * Settings > AI, read-only: providers, keys and switches are set on the computer that runs
 * the engine (docs/ai-plan.md section 7), since a remote device may not point the engine at
 * a URL or opt accounts in.
 */
export function AiSettingsView() {
  const features = useEngineFeatures();
  const settings = useAiSettings();
  const { data: accounts = [] } = useAccounts();

  if (!features?.has(EngineFeature.Ai)) {
    return (
      <SettingsPage title="AI" description="Summaries, reply suggestions and triage.">
        <p className="text-sm text-muted-foreground">This engine has no AI support yet.</p>
      </SettingsPage>
    );
  }

  return (
    <SettingsPage
      title="AI"
      description="The engine calls the model; this browser never does. Set up providers, keys and accounts in the desktop app, Preferences > AI."
    >
      <SettingsSection title="Features">
        {AI_FEATURES.map((f) => (
          <FeatureRow key={f} feature={f} />
        ))}
      </SettingsSection>

      <SettingsSection title="Providers">
        {settings.providers.length === 0 ? (
          <SettingRow label="None set up" description="Add one in the desktop app." />
        ) : (
          settings.providers.map((p) => <ProviderRow key={p.id} provider={p} />)
        )}
      </SettingsSection>

      <SettingsSection title="Accounts" description="Mail from these accounts may go to a model.">
        {accounts.map((a) => (
          <SettingRow key={a.id} label={a.label || a.emailAddress}>
            <Status tone={settings.accounts.includes(a.id) ? 'success' : 'muted'}>
              {settings.accounts.includes(a.id) ? 'On' : 'Off'}
            </Status>
          </SettingRow>
        ))}
        <SettingRow
          label="Include email addresses"
          description="Otherwise the model sees names, and [address] where there is none."
        >
          <span className="text-xs text-muted-foreground">
            {settings.includeAddresses ? 'Yes' : 'No'}
          </span>
        </SettingRow>
      </SettingsSection>

      <SettingsSection title="What leaves the engine">
        <table className="w-full text-left text-xs">
          <thead className="text-muted-foreground">
            <tr className="border-b border-hairline">
              <th className="px-4 py-2 font-medium">Feature</th>
              <th className="px-4 py-2 font-medium">Sent to the provider</th>
              <th className="px-4 py-2 font-medium">When</th>
            </tr>
          </thead>
          <tbody>
            {PRIVACY.map((r) => (
              <tr key={r.feature} className="border-b border-hairline last:border-0">
                <td className="px-4 py-2 font-medium">{r.feature}</td>
                <td className="px-4 py-2 text-muted-foreground">{r.sent}</td>
                <td className="px-4 py-2 text-muted-foreground">{r.when}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="px-4 py-2 text-xs text-muted-foreground">
          Never sent: attachments (only file names), Spam and Trash, drafts, and encrypted mail.
        </p>
      </SettingsSection>

      <Usage names={new Map(settings.providers.map((p) => [p.id, providerName(p)]))} />
    </SettingsPage>
  );
}

function FeatureRow({ feature }: { feature: AiFeature }) {
  const settings = useAiSettings();
  const ai = useAiAvailability(feature);
  const label = FEATURE_LABEL[feature];
  const on = settings.features[feature].enabled;
  const description = ai.provider
    ? `${label.description} ${providerName(ai.provider)}, ${ai.provider.model || 'no model'}.`
    : label.description;
  return (
    <SettingRow label={label.title} description={description}>
      {ai.available ? (
        <Status tone="success">On</Status>
      ) : (
        <span title={unavailableMessage(ai.reason)}>
          <Status tone={on ? 'warning' : 'muted'}>
            {!on
              ? 'Off'
              : ai.reason === 'needs-engine-key'
                ? 'Needs a key on the engine'
                : 'Unavailable'}
          </Status>
        </span>
      )}
    </SettingRow>
  );
}

function ProviderRow({ provider }: { provider: AiProvider }) {
  const client = useEngine();
  const test = useMutation({
    mutationFn: async () => {
      await ensureConnected(client);
      return client.call('ai.test', { provider: provider.id });
    },
  });
  const result = test.isSuccess
    ? `Answered in ${test.data.latencyMs} ms (${test.data.model}).`
    : test.isError
      ? errorMessage(test.error, providerName(provider))
      : null;
  return (
    <SettingRow
      label={providerName(provider)}
      description={
        <>
          {KINDS[provider.kind].name} · {provider.model || 'no model'} · {destinationNote(provider)}
          {result && (
            <span role="status" className="block">
              {result}
            </span>
          )}
        </>
      }
    >
      <Button size="xs" variant="outline" disabled={test.isPending} onClick={() => test.mutate()}>
        {test.isPending ? 'Checking…' : 'Check'}
      </Button>
    </SettingRow>
  );
}

function Usage({ names }: { names: Map<string, string> }) {
  const client = useEngine();
  const { data } = useQuery({
    queryKey: aiKeys.usage(),
    queryFn: async () => {
      await ensureConnected(client);
      return client.call('ai.usage', { days: 30 });
    },
  });
  const totals = new Map<string, { input: number; output: number; requests: number }>();
  for (const r of (data ?? []) as AiUsageRow[]) {
    const t = totals.get(r.provider) ?? { input: 0, output: 0, requests: 0 };
    t.input += r.inputTokens;
    t.output += r.outputTokens;
    t.requests += r.requests;
    totals.set(r.provider, t);
  }
  return (
    <SettingsSection title="Last 30 days" description="Tokens as the providers count them.">
      {totals.size === 0 ? (
        <SettingRow label="No requests yet" />
      ) : (
        [...totals].map(([id, t]) => (
          <SettingRow key={id} label={names.get(id) ?? id}>
            <span className="font-mono text-2xs text-muted-foreground tabular">
              {t.requests} requests · {t.input.toLocaleString()} in · {t.output.toLocaleString()}{' '}
              out
            </span>
          </SettingRow>
        ))
      )}
    </SettingsSection>
  );
}
