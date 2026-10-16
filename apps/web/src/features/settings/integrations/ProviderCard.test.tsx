import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { renderWithEngine } from '../test-utils';
import { ProviderCard } from './ProviderCard';
import { providerDef } from './providers';
import { getIntegrations, resetIntegrations } from './store';

beforeEach(() => resetIntegrations());

describe('ProviderCard', () => {
  it('saves a key without storing it in the browser', async () => {
    renderWithEngine(<ProviderCard def={providerDef('deepl')} />);
    const card = within(screen.getByRole('form', { name: 'DeepL settings' }));
    const user = userEvent.setup();

    await user.click(card.getByRole('switch'));
    expect(card.getByText('Needs setup')).toBeInTheDocument();

    await user.type(card.getByLabelText(/API key/), 'deepl-secret');
    await user.click(card.getByRole('button', { name: 'Save' }));

    expect(card.getByText('On')).toBeInTheDocument();
    expect(getIntegrations().secrets.deepl).toEqual(['apiKey']);
    expect(localStorage.getItem('tern.integrations')).not.toContain('deepl-secret');
    expect(card.getByLabelText(/API key/)).toHaveValue('');
  });

  it('shows the result of a connection test', async () => {
    renderWithEngine(<ProviderCard def={providerDef('pdl')} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(await screen.findByText('An API key is required')).toBeInTheDocument();

    await user.type(screen.getByLabelText(/API key/), 'pdl-key');
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(await screen.findByText('Connected')).toBeInTheDocument();
  });
});
