// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
What a window shows while its reads go to the engine and the engine isn't there:
in place of an empty list, and as a notification over
lists that keep their last rows. A short wait (a respawn, a reconnect) shows nothing.
*/
import React, { useEffect, useState } from 'react';
import { ipcRenderer } from 'electron';
import { localized } from '../intl/localize';
import EngineReads, {
  ENGINE_READS_KEY,
  EngineReads as EngineReadsType,
} from '../data/database/engine-reads';
import { SupervisorChannel } from '../engine/channels';
import Notification from './notification';

export const SHOW_WAITING_AFTER_MS = 2000;

type Shown = 'waiting' | 'gave-up' | null;

export function useEngineUnavailable(reads: EngineReadsType = EngineReads): Shown {
  const [shown, setShown] = useState<Shown>(() =>
    reads.unavailable() === 'gave-up' ? 'gave-up' : null
  );
  useEffect(() => {
    let timer: NodeJS.Timeout = null;
    const update = () => {
      clearTimeout(timer);
      timer = null;
      const next = reads.unavailable();
      if (next !== 'waiting') {
        setShown(next);
        return;
      }
      // After Try Again the state stays up while the engine starts; otherwise wait a moment.
      setShown((current) => (current ? 'waiting' : current));
      timer = setTimeout(() => setShown(reads.unavailable()), SHOW_WAITING_AFTER_MS);
    };
    reads.on('state', update);
    const flag = AppEnv.config.onDidChange(ENGINE_READS_KEY, update);
    update();
    return () => {
      clearTimeout(timer);
      reads.removeListener('state', update);
      flag.dispose();
    };
  }, [reads]);
  return shown;
}

export function retryEngine() {
  ipcRenderer.invoke(SupervisorChannel.RetryEngine).catch((err) => {
    console.warn(`Could not ask the main process to start the sync engine: ${err}`);
  });
}

function texts(shown: Shown) {
  return shown === 'gave-up'
    ? {
        title: localized('The sync engine stopped'),
        subtitle: localized(
          'Tern restarted it several times and gave up. Mail can’t be read until it runs.'
        ),
      }
    : {
        title: localized('Waiting for the sync engine'),
        subtitle: localized('Lists show what was loaded before it went away.'),
      };
}

// For an empty list: the engine state, or `children` while reads work.
export function EngineUnavailableState({
  reads,
  children = null,
}: {
  reads?: EngineReadsType;
  children?: React.ReactNode;
}) {
  const shown = useEngineUnavailable(reads);
  if (!shown) return <>{children}</>;
  const { title } = texts(shown);
  return (
    <div className="perspective-empty-state engine-unavailable-state">
      <div className="message">
        {title}
        <div>
          <button className="btn" onClick={retryEngine}>
            {localized('Try Again')}
          </button>
        </div>
      </div>
    </div>
  );
}

export function EngineUnavailableNotification({ reads }: { reads?: EngineReadsType }) {
  const shown = useEngineUnavailable(reads);
  if (!shown) return <span />;
  const { title, subtitle } = texts(shown);
  return (
    <Notification
      priority="4"
      isError={shown === 'gave-up'}
      title={title}
      subtitle={subtitle}
      actions={[{ label: localized('Try Again'), fn: retryEngine }]}
      icon="volstead-error.png"
    />
  );
}
EngineUnavailableNotification.displayName = 'EngineUnavailableNotification';
