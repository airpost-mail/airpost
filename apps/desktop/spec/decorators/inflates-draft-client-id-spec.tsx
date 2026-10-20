import React from 'react';
import ReactDOM from 'react-dom';
import * as Actions from '../../src/flux/actions';
import DraftStore from '../../src/flux/stores/draft-store';
import { Message } from '../../src/flux/models/message';
import InflatesDraftClientId from '../../src/components/decorators/inflates-draft-client-id';

class Dummy extends React.Component<Record<string, unknown>> {
  static displayName = 'Dummy';
  render() {
    return <div className="dummy" />;
  }
}

const Inflated = InflatesDraftClientId(Dummy);

describe('InflatesDraftClientId', function () {
  let container: HTMLDivElement;
  const draft = new Message({ id: 'draft-1', headerMessageId: 'hmid-1', draft: true });
  (draft as any).pristine = true;

  beforeEach(() => {
    container = document.createElement('div');
    const session = {
      headerMessageId: 'hmid-1',
      draft: () => draft,
      listen: () => () => {},
      changes: { commit: () => Promise.resolve() },
    };
    spyOn(DraftStore, 'sessionForClientId').andReturn(Promise.resolve(session as any));
    spyOn(Actions, 'destroyDraft');
  });

  const mountAndUnmount = (props: Record<string, unknown>) => {
    let ready = false;
    ReactDOM.render(
      <Inflated headerMessageId="hmid-1" onDraftReady={() => (ready = true)} {...props} />,
      container
    );
    waitsFor(() => ready);
    runs(() => {
      ReactDOM.unmountComponentAtNode(container);
    });
  };

  it('destroys a pristine draft on unmount', function () {
    mountAndUnmount({});
    runs(() => expect(Actions.destroyDraft).toHaveBeenCalledWith(draft));
  });

  it('keeps a pristine draft on unmount when keepPristineDraft is set', function () {
    mountAndUnmount({ keepPristineDraft: true });
    runs(() => expect(Actions.destroyDraft).not.toHaveBeenCalled());
  });
});
