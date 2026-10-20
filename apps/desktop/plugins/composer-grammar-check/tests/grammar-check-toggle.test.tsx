// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { CapabilityStore, GrammarCheckPluginAPI, Message } from 'mailspring-exports';
import { GrammarCheckToggle } from '../lib/toggle';
import { GrammarCheckStore } from '../lib/store';

describe('GrammarCheckToggle', () => {
  const draft = new Message({ draft: true, headerMessageId: 'grammar-draft' });

  afterEach(() => cleanup());

  const renderToggle = () => render(<GrammarCheckToggle draft={draft} session={null} />).container;

  it('is a disabled kit toolbar button with the reason as its name when unavailable', () => {
    spyOn(GrammarCheckStore, 'isAvailable').andReturn(false);
    spyOn(CapabilityStore, 'unavailableMessage').andReturn('Set up a provider');
    const container = renderToggle();
    const button = container.querySelector('.btn-grammar-check') as HTMLButtonElement;
    expect(button.getAttribute('data-slot')).toBe('button');
    expect(button.getAttribute('data-variant')).toBe('toolbar');
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('aria-label')).toBe('Set up a provider');
    expect(button.hasAttribute('title')).toBe(false);
    // The tooltip hangs off a wrapper, since a disabled button takes no pointer events.
    expect(button.parentElement.getAttribute('data-slot')).toBe('tooltip-trigger');
  });

  it('shows the state and toggles the setting when available', () => {
    spyOn(GrammarCheckStore, 'isAvailable').andReturn(true);
    spyOn(GrammarCheckStore, 'isEnabled').andReturn(true);
    spyOn(GrammarCheckStore, 'isChecking').andReturn(true);
    spyOn(GrammarCheckStore, 'errorCount').andReturn(2);
    spyOn(GrammarCheckPluginAPI, 'requestInitialCheckForDraft');
    AppEnv.config.set('core.composing.grammarCheck', true);

    const container = renderToggle();
    const button = container.querySelector('.btn-grammar-check') as HTMLButtonElement;
    expect(button.getAttribute('data-slot')).toBe('button');
    expect(button.className).toContain('btn btn-toolbar');
    expect(button.classList.contains('btn-enabled')).toBe(true);
    expect(button.classList.contains('checking')).toBe(true);
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.getAttribute('aria-label')).toBe('Grammar check: 2 issue(s)');
    expect(button.querySelector('.grammar-check-spinner')).not.toBe(null);

    fireEvent.click(button);
    expect(AppEnv.config.get('core.composing.grammarCheck')).toBe(false);
    expect(GrammarCheckPluginAPI.requestInitialCheckForDraft).not.toHaveBeenCalled();
  });

  it('asks for a check when turned on', () => {
    spyOn(GrammarCheckStore, 'isAvailable').andReturn(true);
    spyOn(GrammarCheckStore, 'isEnabled').andReturn(false);
    spyOn(GrammarCheckStore, 'isChecking').andReturn(false);
    spyOn(GrammarCheckStore, 'errorCount').andReturn(0);
    spyOn(GrammarCheckPluginAPI, 'requestInitialCheckForDraft');
    AppEnv.config.set('core.composing.grammarCheck', false);

    const button = renderToggle().querySelector('.btn-grammar-check');
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.getAttribute('aria-label')).toBe('Enable grammar check');
    fireEvent.click(button);
    expect(AppEnv.config.get('core.composing.grammarCheck')).toBe(true);
    expect(GrammarCheckPluginAPI.requestInitialCheckForDraft).toHaveBeenCalledWith('grammar-draft');
  });
});
