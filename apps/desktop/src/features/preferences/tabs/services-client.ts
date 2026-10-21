import { ipcRenderer } from 'electron';
import { localized } from '../../../intl/localize';
import type { StatusKind } from '../../../components/settings';
import {
  GatewayStatus,
  McpHttpStatus,
  ServiceStatus,
  ServicesChannel,
} from '../../../platform/services';

// The panes' side of main/ipc/services.ts.

type Changed = McpHttpStatus | GatewayStatus;

/** Invokes a services channel; null when main has no handler (a window reloaded without a
 * restart) or runs no services (specs). */
export async function invokeServices<T>(channel: string, ...args: unknown[]): Promise<T | null> {
  try {
    return await ipcRenderer.invoke(channel, ...args);
  } catch (err) {
    return null;
  }
}

/** Calls `callback` with each status main broadcasts for `service`; returns the unsubscribe. */
export function onServiceChange<S extends Changed>(
  service: S['service'],
  callback: (status: S) => void
) {
  const listener = (_event: unknown, status: Changed) => {
    if (status && status.service === service) callback(status as S);
  };
  ipcRenderer.on(ServicesChannel.Changed, listener);
  return () => {
    ipcRenderer.removeListener(ServicesChannel.Changed, listener);
  };
}

export function servicePill(status: ServiceStatus): { kind: StatusKind; text: string } {
  switch (status.state) {
    case 'running':
      return { kind: 'ok', text: localized('Running') };
    case 'starting':
      return { kind: 'off', text: localized('Starting') };
    case 'error':
      return { kind: 'bad', text: localized('Not running') };
    default:
      return { kind: 'off', text: localized('Off') };
  }
}
