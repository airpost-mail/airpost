import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RpcError, RpcErrorCode } from '@/engine';
import { renderWithEngine } from '../test-utils';
import { ProviderCard } from './ProviderCard';
import { providerDef } from './providers';
import { getIntegrations, resetIntegrations } from './store';
import { integrationConfig } from './useIntegrationTest';

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
    expect(localStorage.getItem('airpost.integrations')).not.toContain('deepl-secret');
    expect(card.getByLabelText(/API key/)).toHaveValue('');
  });

  it('shows the result of a connection test', async () => {
    renderWithEngine(<ProviderCard def={providerDef('pdl')} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(await screen.findByText('Enter the API key first.')).toBeInTheDocument();

    await user.type(screen.getByLabelText(/API key/), 'pdl-key');
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(await screen.findByText('Connected to api.peopledatalabs.com')).toBeInTheDocument();
  });

  it('shows why the engine’s provider call failed', async () => {
    renderWithEngine(<ProviderCard def={providerDef('libretranslate')} />);
    const user = userEvent.setup();
    const url = screen.getByLabelText(/Server URL/);
    await user.clear(url);
    await user.type(url, 'ftp://translate.example');
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(
      await screen.findByText('The server URL is not a valid http(s) URL'),
    ).toBeInTheDocument();
  });

  it('says a device without admin can’t test providers', async () => {
    const { engine } = renderWithEngine(<ProviderCard def={providerDef('pdl')} />);
    vi.spyOn(engine, 'call').mockRejectedValue(
      new RpcError({
        code: RpcErrorCode.NotAllowedRelayed,
        message: 'not allowed for a relayed client',
      }),
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText(/API key/), 'pdl-key');
    await user.click(screen.getByRole('button', { name: 'Test' }));
    expect(
      await screen.findByText('Testing providers from the web needs an admin device.'),
    ).toBeInTheDocument();
  });
});

describe('integrationConfig', () => {
  it('names providers as the engine does', () => {
    expect(integrationConfig('pdl', { apiKey: 'k' })).toEqual({
      provider: 'peopledatalabs',
      apiKey: 'k',
    });
  });
});
