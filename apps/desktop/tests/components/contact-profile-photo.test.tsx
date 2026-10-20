import React from 'react';
import ReactDOMServer from 'react-dom/server';

import { ContactProfilePhoto } from '../../src/components/contact-profile-photo';
import { Contact } from '../../src/data/models/contact';

const render = () =>
  ReactDOMServer.renderToStaticMarkup(
    <ContactProfilePhoto
      contact={new Contact({ email: 'friend@example.com', name: 'Ada Lovelace' })}
      loading={false}
      avatar={null}
    />
  );

describe('ContactProfilePhoto', function () {
  it('does not load Gravatar by default', () => {
    expect(AppEnv.config.get('core.reading.loadGravatar')).toBe(false);
    const html = render();
    expect(html).not.toContain('gravatar');
    expect(html).toContain('AL');
  });

  it('loads Gravatar when the setting is on', () => {
    AppEnv.config.set('core.reading.loadGravatar', true);
    expect(render()).toContain('https://www.gravatar.com/avatar/');
  });
});
