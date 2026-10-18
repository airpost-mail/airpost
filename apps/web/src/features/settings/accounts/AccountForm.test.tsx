import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { renderWithEngine } from '../test-utils';
import { AccountForm } from './AccountForm';
import { OTHER_PRESET, presetById } from './presets';

describe('AccountForm', () => {
  beforeAll(() => {
    // jsdom lacks what the Radix select calls when it opens.
    Element.prototype.hasPointerCapture ??= () => false;
    Element.prototype.releasePointerCapture ??= () => {};
    Element.prototype.scrollIntoView ??= () => {};
  });

  it('tests and adds an account through the engine', async () => {
    const onAdded = vi.fn();
    const { engine } = renderWithEngine(
      <AccountForm preset={presetById('fastmail')!} onAdded={onAdded} />,
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Email address'), 'new@fastmail.com');
    await user.type(screen.getByLabelText('Password'), 'app-password');
    await user.click(screen.getByRole('button', { name: 'Add account' }));

    await waitFor(() => expect(onAdded).toHaveBeenCalled());
    const added = [...engine.store.accounts.values()].find(
      (a) => a.emailAddress === 'new@fastmail.com',
    );
    expect(added?.settings).toMatchObject({
      imap_host: 'imap.fastmail.com',
      imap_username: 'new@fastmail.com',
    });
  });

  it('shows field errors and opens server settings when they are missing', async () => {
    renderWithEngine(<AccountForm preset={OTHER_PRESET} onAdded={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Add account' }));
    expect(screen.getByText('Enter an email address.')).toBeInTheDocument();
    expect(screen.getAllByText('Required.')).toHaveLength(2);
  });

  it('needs "Send password unencrypted" for Security None on a remote host', async () => {
    const onAdded = vi.fn();
    const { engine } = renderWithEngine(<AccountForm preset={OTHER_PRESET} onAdded={onAdded} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Email address'), 'me@okafor.studio');
    await user.type(screen.getByLabelText('Password'), 'pw');
    const imap = screen.getByRole('group', { name: 'IMAP' });
    await user.click(within(imap).getByLabelText('Security'));
    await user.click(await screen.findByRole('option', { name: 'None' }));

    await user.click(screen.getByRole('button', { name: 'Add account' }));
    expect(
      screen.getByText(/confirm that your password can be sent unencrypted/),
    ).toBeInTheDocument();
    expect(onAdded).not.toHaveBeenCalled();

    await user.click(within(imap).getByRole('checkbox', { name: 'Send password unencrypted' }));
    await user.click(screen.getByRole('button', { name: 'Add account' }));
    await waitFor(() => expect(onAdded).toHaveBeenCalled());
    const added = [...engine.store.accounts.values()].find(
      (a) => a.emailAddress === 'me@okafor.studio',
    );
    expect(added?.settings).toMatchObject({
      imap_security: 'none',
      imap_allow_cleartext: 'imap.okafor.studio',
    });
  });

  it('asks again after the host changes', async () => {
    renderWithEngine(<AccountForm preset={OTHER_PRESET} onAdded={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Email address'), 'me@okafor.studio');
    const imap = screen.getByRole('group', { name: 'IMAP' });
    await user.click(within(imap).getByLabelText('Security'));
    await user.click(await screen.findByRole('option', { name: 'None' }));
    const box = within(imap).getByRole('checkbox', { name: 'Send password unencrypted' });
    await user.click(box);
    expect(box).toBeChecked();
    await user.type(within(imap).getByLabelText('Server'), '.example');
    expect(
      within(imap).getByRole('checkbox', { name: 'Send password unencrypted' }),
    ).not.toBeChecked();
  });
});
