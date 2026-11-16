import { Link } from '@tanstack/react-router';
import { Cloud, Laptop, Plug } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useCapabilities, type Capability, type CapabilitySource } from './capabilities';
import { SettingRow, SettingsSection } from './components/SettingsSection';
import { SettingsPage } from './components/SettingsPage';
import { Status } from './components/StatusDot';
import { providerDef } from './integrations/providers';

const SOURCES: { source: CapabilitySource; title: string; note: string; icon: LucideIcon }[] = [
  {
    source: 'local',
    title: 'Local',
    note: 'Run by your engine. Always on.',
    icon: Laptop,
  },
  {
    source: 'integration',
    title: 'Integration',
    note: 'On when you set up a provider.',
    icon: Plug,
  },
  {
    source: 'server',
    title: 'Server',
    note: 'Need a Airpost server to send, receive or store on your behalf.',
    icon: Cloud,
  },
];

export function CapabilitiesSettings() {
  const caps = useCapabilities();
  return (
    <SettingsPage
      title="Capabilities"
      description="What this Airpost can do, and where it comes from."
    >
      {SOURCES.map(({ source, title, note, icon: Icon }) => (
        <SettingsSection
          key={source}
          title={title}
          description={note}
          action={<Icon className="size-4 text-muted-foreground" aria-hidden />}
        >
          {caps
            .filter((c) => c.source === source)
            .map((c) => (
              <SettingRow key={c.name} label={c.title} description={c.description}>
                <CapabilityStatus cap={c} />
              </SettingRow>
            ))}
        </SettingsSection>
      ))}
    </SettingsPage>
  );
}

function CapabilityStatus({ cap }: { cap: Capability }) {
  if (cap.available) {
    return (
      <Status tone="success">
        {cap.provider ? `Via ${providerDef(cap.provider).name}` : 'Available'}
      </Status>
    );
  }
  if (cap.reason === 'needs-integration') {
    return (
      <Link
        to="/settings/integrations"
        className="rounded-sm text-xs text-primary underline-offset-4 hover:underline"
      >
        Set up
      </Link>
    );
  }
  return <Status tone="muted">Needs a Airpost server</Status>;
}
