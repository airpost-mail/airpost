// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import SignaturePhotoPicker from '../../src/features/composer-signature/photo-picker';
import SignatureTemplatePicker from '../../src/features/composer-signature/template-picker';
import Templates from '../../src/features/composer-signature/templates';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait } from '../support/kit-helpers';

const items = () =>
  Array.from(getPortalContainer().querySelectorAll('[data-slot="select-item"]')) as HTMLElement[];

describe('SignaturePhotoPicker', function () {
  beforeEach(() => jasmine.unspy(window, 'setTimeout'));
  afterEach(() => cleanup());

  const renderPicker = (photoURL: string, onChange = jasmine.createSpy('onChange')) => {
    const { container } = render(
      <SignaturePhotoPicker data={{ photoURL }} resolvedURL={photoURL} onChange={onChange} />
    );
    return { container, onChange, trigger: container.querySelector('#photoURL') as HTMLElement };
  };

  it('shows the source in a kit select labelled Picture', () => {
    const { container, trigger } = renderPicker('');
    expect(trigger.getAttribute('data-slot')).toBe('select-trigger');
    expect(container.querySelector('label[for="photoURL"]').textContent).toBe('Picture');
    expect(trigger.textContent).toContain('None');
    cleanup();
    expect(renderPicker('gravatar').trigger.textContent).toContain('Gravatar Profile Photo');
  });

  it('reports the picked source as a photoURL change, with None as an empty value', async () => {
    const { trigger, onChange } = renderPicker('gravatar');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    await wait(50);
    expect(items().map((i) => i.textContent)).toEqual([
      'None',
      'Gravatar Profile Photo',
      'Custom Image…',
    ]);
    fireEvent.keyDown(items()[0], { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith({ target: { id: 'photoURL', value: '' } });

    fireEvent.keyDown(trigger, { key: 'Enter' });
    await wait(50);
    fireEvent.keyDown(items()[2], { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith({ target: { id: 'photoURL', value: 'custom' } });
  });

  it('shows a URL field for a custom picture', () => {
    const { container } = renderPicker('https://example.com/me.png');
    expect(container.querySelector('#photoURL').textContent).toContain('Custom Image…');
    expect((container.querySelector('input[type="url"]') as HTMLInputElement).value).toBe(
      'https://example.com/me.png'
    );
  });
});

describe('SignatureTemplatePicker', function () {
  afterEach(() => cleanup());

  const renderPicker = (templateName: string | undefined) => {
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(
      <SignatureTemplatePicker
        resolvedData={{ templateName, name: 'Ada', email: 'ada@example.com' }}
        onChange={onChange}
      />
    );
    const options = Array.from(container.querySelectorAll('.option')) as HTMLElement[];
    return { container, onChange, options };
  };

  it('is a kit radio group with a tile per template and Raw HTML', () => {
    const { container, options } = renderPicker(Templates[1].name);
    const group = container.querySelector('.signature-template-picker');
    expect(group.getAttribute('data-slot')).toBe('radio-group');
    expect(group.getAttribute('role')).toBe('radiogroup');
    expect(options.length).toBe(Templates.length + 1);
    expect(options.map((o) => o.getAttribute('aria-checked'))).toEqual(
      options.map((o, i) => String(i === 1))
    );
    expect(options.map((o) => o.classList.contains('active'))).toEqual(
      options.map((o, i) => i === 1)
    );
  });

  it('checks Raw HTML when there is no template', () => {
    const { options } = renderPicker(undefined);
    const raw = options[options.length - 1];
    expect(raw.textContent).toBe('Raw HTML');
    expect(raw.getAttribute('data-state')).toBe('checked');
    expect(raw.classList.contains('active')).toBe(true);
  });

  it('reports the picked template, and no template for Raw HTML', () => {
    const { options, onChange } = renderPicker(Templates[0].name);
    fireEvent.click(options[2]);
    expect(onChange).toHaveBeenCalledWith({
      target: { id: 'templateName', value: Templates[2].name },
    });
    fireEvent.click(options[options.length - 1]);
    expect(onChange).toHaveBeenCalledWith({ target: { id: 'templateName', value: undefined } });
  });
});
