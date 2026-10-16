import { SettingsPage } from '../components/SettingsPage';
import { SettingsSection } from '../components/SettingsSection';
import { ProviderCard } from './ProviderCard';
import { KIND_LABEL, providersFor, type IntegrationKind } from './providers';

const KINDS: { kind: IntegrationKind; note?: string }[] = [
  { kind: 'translation', note: 'One translation provider is on at a time.' },
  { kind: 'grammar' },
  { kind: 'enrichment' },
];

export function IntegrationsSettings() {
  return (
    <SettingsPage
      title="Integrations"
      description="Services you bring yourself. The engine calls them; this browser never does, and keys are never stored here."
    >
      {KINDS.map(({ kind, note }) => (
        <SettingsSection key={kind} title={KIND_LABEL[kind]} description={note}>
          {providersFor(kind).map((def) => (
            <ProviderCard key={def.id} def={def} />
          ))}
        </SettingsSection>
      ))}
    </SettingsPage>
  );
}
