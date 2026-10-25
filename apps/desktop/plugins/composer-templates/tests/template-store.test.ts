import { DraftStore, Engine, Message } from 'mailspring-exports';
import TemplateStore from '../lib/store';
import { Dialogs } from '../../../src/platform/app-api';

const TEMPLATE = {
  id: 'Follow Up.html',
  name: 'Follow Up',
  subject: 'Following up on our call',
};

type DraftProps = ConstructorParameters<typeof Message>[0];

// Engine.request answering the templates methods; `calls` records them.
function fakeEngine(get: { subject: string; body: string } = { subject: '', body: '' }) {
  const calls: [string, any][] = [];
  spyOn(Engine, 'request').andCallFake((method: string, params: any) => {
    calls.push([method, params]);
    if (method === 'templates.list') return Promise.resolve({ templates: [TEMPLATE] });
    if (method === 'templates.get') {
      return Promise.resolve({ template: { ...TEMPLATE, ...get } });
    }
    return Promise.resolve({});
  });
  return calls;
}

describe('TemplateStore', function templateStore() {
  describe('inserting a template into a draft', () => {
    const insert = async (
      draftProps: DraftProps,
      contents = { subject: 'Following up on our call', body: '<div>Hey!</div>' }
    ) => {
      const draft = new Message({
        draft: true,
        pristine: true,
        subject: '',
        body: '',
        ...draftProps,
      });
      const session = { draft: () => draft, changes: { add: jasmine.createSpy('add') } };
      fakeEngine(contents);
      spyOn(DraftStore, 'sessionForClientId').andReturn(Promise.resolve(session));
      await (TemplateStore as any)._onInsertTemplateId({
        templateId: TEMPLATE.id,
        headerMessageId: 'draft-1',
      });
      return session.changes.add.calls.length ? session.changes.add.mostRecentCall.args[0] : null;
    };

    beforeEach(() => {
      (TemplateStore as any)._items = [TEMPLATE];
      spyOn(TemplateStore as any, '_displayDialog').andReturn(Promise.resolve(true));
    });

    it("fills in the draft's subject and body when the draft has no subject", async () => {
      const changes = await insert({});
      expect(changes.subject).toBe('Following up on our call');
      expect(changes.body).toBe('<div>Hey!</div>');
      expect((TemplateStore as any)._displayDialog).not.toHaveBeenCalled();
    });

    it('leaves the subject alone when the template does not have one', async () => {
      const changes = await insert({}, { subject: '', body: '<div>Hey!</div>' });
      expect(changes.subject).toBe(undefined);
      expect(changes.body).toBe('<div>Hey!</div>');
    });

    it('never replaces the subject of a reply', async () => {
      const changes = await insert({
        subject: 'Re: Lunch tomorrow',
        replyToHeaderMessageId: 'other-message',
      });
      expect(changes.subject).toBe(undefined);
      expect((TemplateStore as any)._displayDialog).not.toHaveBeenCalled();
    });

    it('never replaces the subject of a forward', async () => {
      const changes = await insert({
        subject: 'Fwd: Lunch tomorrow',
        forwardedHeaderMessageId: 'other-message',
      });
      expect(changes.subject).toBe(undefined);
      expect((TemplateStore as any)._displayDialog).not.toHaveBeenCalled();
    });

    it('asks before replacing a subject the draft already has', async () => {
      const changes = await insert({ subject: 'Lunch tomorrow' });
      expect((TemplateStore as any)._displayDialog).toHaveBeenCalled();
      expect(changes.subject).toBe('Following up on our call');
    });

    it('does not ask when the draft subject already matches the template', async () => {
      const changes = await insert({ subject: 'Following up on our call' });
      expect((TemplateStore as any)._displayDialog).not.toHaveBeenCalled();
      expect(changes.subject).toBe('Following up on our call');
    });

    it('makes no changes when the user cancels the replacement', async () => {
      (TemplateStore as any)._displayDialog.andReturn(Promise.resolve(false));
      const changes = await insert({ subject: 'Lunch tomorrow' });
      expect(changes).toBe(null);
    });

    it('inserts the body above the signature and quoted text', async () => {
      const changes = await insert({
        body: '<div>draft</div><signature>Ben</signature>',
        pristine: false,
      });
      expect(changes.body).toBe('<div>Hey!</div><signature>Ben</signature>');
    });
  });

  describe('creating a template from a draft', () => {
    it('saves the draft once its changes are, and the engine makes the template of it', async () => {
      const draft = new Message({ draft: true, id: 'm1', accountId: 'a1', subject: 'Hi' });
      const commit = jasmine.createSpy('commit').andReturn(Promise.resolve());
      spyOn(DraftStore, 'sessionForClientId').andReturn(
        Promise.resolve({ draft: () => draft, changes: { commit } })
      );
      spyOn(TemplateStore as any, '_onShowTemplates');
      const calls = fakeEngine();
      await (TemplateStore as any)._onCreateTemplateFromDraft('draft-1');
      expect(commit).toHaveBeenCalled();
      expect(calls[0]).toEqual(['templates.save', { accountId: 'a1', messageId: 'm1' }]);
      expect(calls[1][0]).toBe('templates.list');
      expect((TemplateStore as any)._onShowTemplates).toHaveBeenCalled();
    });

    it("shows the engine's reason when it can't", async () => {
      const draft = new Message({ draft: true, id: 'm1', accountId: 'a1', subject: '' });
      spyOn(DraftStore, 'sessionForClientId').andReturn(
        Promise.resolve({ draft: () => draft, changes: { commit: () => Promise.resolve() } })
      );
      spyOn(Engine, 'request').andReturn(
        Promise.reject(
          new Error('invalid params: Give your draft a subject to name your template.')
        )
      );
      spyOn(AppEnv, 'showErrorDialog');
      await (TemplateStore as any)._onCreateTemplateFromDraft('draft-1');
      expect((AppEnv.showErrorDialog as any).mostRecentCall.args[0].message).toBe(
        'Give your draft a subject to name your template.'
      );
    });
  });

  describe('renaming a template', () => {
    it('saves its contents under the new name and deletes the old file', async () => {
      (TemplateStore as any)._items = [{ ...TEMPLATE }];
      const calls = fakeEngine({ subject: 'S', body: '<p>B</p>' });
      await (TemplateStore as any)._onRenameTemplate(TEMPLATE.name, 'Checking in');
      expect(calls.slice(0, 3)).toEqual([
        ['templates.get', { name: TEMPLATE.id }],
        ['templates.save', { name: 'Checking in', subject: 'S', body: '<p>B</p>', replace: true }],
        ['templates.delete', { name: TEMPLATE.id }],
      ]);
    });
  });

  describe('deleting a template', () => {
    let calls: [string, any][];
    const deletes = () => calls.filter(([m]) => m === 'templates.delete');

    beforeEach(() => {
      (TemplateStore as any)._items = [TEMPLATE];
      calls = fakeEngine();
    });

    it('deletes the file once the app-modal confirmation says Delete', async () => {
      const ask = spyOn(Dialogs, 'ask').andReturn(Promise.resolve(0));
      await (TemplateStore as any)._onDeleteTemplate(TEMPLATE.name);
      expect(ask.mostRecentCall.args[1]).toBe('none');
      expect(deletes()).toEqual([['templates.delete', { name: TEMPLATE.id }]]);
    });

    it('keeps the file when the confirmation is cancelled', async () => {
      spyOn(Dialogs, 'ask').andReturn(Promise.resolve(1));
      await (TemplateStore as any)._onDeleteTemplate(TEMPLATE.name);
      expect(deletes().length).toBe(0);
    });

    it('shows one confirmation at a time and cancels the second request', async () => {
      let answer: (index: number) => void = () => {};
      const ask = spyOn(Dialogs, 'ask').andReturn(new Promise<number>((r) => (answer = r)));
      const first = (TemplateStore as any)._onDeleteTemplate(TEMPLATE.name);
      await (TemplateStore as any)._onDeleteTemplate(TEMPLATE.name);
      expect(ask.calls.length).toBe(1);
      expect(deletes().length).toBe(0);
      answer(0);
      await first;
      expect(deletes().length).toBe(1);
    });
  });
});
