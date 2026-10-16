import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { renderWithEngine } from '../test-utils';
import { AccountForm } from './AccountForm';
import { OTHER_PRESET, presetById } from './presets';

describe('AccountForm', () => {
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
});
