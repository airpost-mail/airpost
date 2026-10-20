import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { ipcRenderer } from 'electron';
import AccountChoosePage from '../../src/features/onboarding/pages/account-choose';
import { ImportFromMailspringButton } from '../../src/components/mailspring-import';
import { MailspringImportChannel } from '../../src/engine/channels';

describe('Import from Mailspring in onboarding', function () {
  let finish: (outcome: string) => void;

  const render = async (available: boolean) => {
    spyOn(ipcRenderer, 'invoke').andCallFake((channel: string) =>
      channel === MailspringImportChannel.Available
        ? Promise.resolve(available)
        : new Promise((resolve) => {
            finish = resolve;
          })
    );
    let renderer: TestRenderer.ReactTestRenderer;
    await act(async () => {
      renderer = TestRenderer.create(<AccountChoosePage />);
    });
    return renderer;
  };

  const buttons = (renderer: TestRenderer.ReactTestRenderer) =>
    renderer.root.findAllByType(ImportFromMailspringButton);

  it('is offered when there is a Mailspring profile', async function () {
    const renderer = await render(true);
    expect(buttons(renderer).length).toBe(1);
    expect(ipcRenderer.invoke).toHaveBeenCalledWith(MailspringImportChannel.Available);
  });

  it('is not shown otherwise', async function () {
    const renderer = await render(false);
    expect(buttons(renderer).length).toBe(0);
  });

  it('asks main to import, and stays disabled until main is done', async function () {
    const renderer = await render(true);
    const button = () => buttons(renderer)[0].findByType('button');
    await act(async () => {
      button().props.onClick();
    });
    expect(ipcRenderer.invoke).toHaveBeenCalledWith(MailspringImportChannel.Start);
    expect(button().props.disabled).toBe(true);
    await act(async () => {
      finish('cancelled');
    });
    expect(button().props.disabled).toBeFalsy();
  });
});
