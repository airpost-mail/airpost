// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import { ipcRenderer } from 'electron';
import { localized } from '../intl/localize';
import { Button, ButtonProps } from './kit/button';
import { MailspringImportChannel, MailspringImportOutcome } from '../engine/channels';

/*
The window side of Import from Mailspring, for onboarding and Preferences > Accounts. The main
process finds the Mailspring profile, shows the confirmation and the result, and does the import
(main/profile/mailspring-import-flow.ts).
*/

// Whether there is a Mailspring profile to import from; false until main answers.
export function useMailspringProfile(): boolean {
  const [available, setAvailable] = React.useState(false);
  React.useEffect(() => {
    let live = true;
    ipcRenderer.invoke(MailspringImportChannel.Available).then(
      (value: boolean) => live && setAvailable(!!value),
      (err) => console.warn(`Could not look for a Mailspring profile: ${err}`)
    );
    return () => {
      live = false;
    };
  }, []);
  return available;
}

export function startMailspringImport(): Promise<MailspringImportOutcome> {
  return ipcRenderer.invoke(MailspringImportChannel.Start);
}

// "Import from Mailspring", disabled while the import runs.
export function ImportFromMailspringButton(props: Omit<ButtonProps, 'onClick'>) {
  const [busy, setBusy] = React.useState(false);
  const mounted = React.useRef(true);
  React.useEffect(
    () => () => {
      mounted.current = false;
    },
    []
  );
  const onClick = () => {
    setBusy(true);
    startMailspringImport()
      .catch((err) => console.error(`Import from Mailspring failed: ${err}`))
      .then(() => mounted.current && setBusy(false));
  };
  return (
    <Button {...props} disabled={busy || props.disabled} onClick={onClick}>
      {props.children || localized('Import from Mailspring')}
    </Button>
  );
}
