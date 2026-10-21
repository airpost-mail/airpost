import React from 'react';
import DraftStore from '../../data/drafts/store';
import * as Actions from '../../data/actions';
import * as Utils from '../../data/models/utils';
import { DraftEditingSession, MessageWithEditorState } from '../../data/drafts/editing-session';

interface InflatesDraftClientIdProps {
  headerMessageId: string;
  onDraftReady?: () => void;
  // A preview that swaps drafts as the selection moves must not destroy the pristine ones
  // it shows: the user never opened them for writing.
  keepPristineDraft?: boolean;
}

interface InflatesDraftClientIdState {
  // Both null until the session for headerMessageId is ready
  draft: MessageWithEditorState | null;
  session: DraftEditingSession | null;
  _lastHeaderMessageId: string;
}

type Props = InflatesDraftClientIdProps & React.HTMLProps<HTMLElement>;

// Plugin API: takes any component, as the implicitly-any parameter did before strict mode. It
// renders with the outer props plus `draft` and `session`.
function InflatesDraftClientId(
  ComposedComponent: React.ComponentType<any>
): React.ComponentType<Props> {
  return class extends React.Component<Props, InflatesDraftClientIdState> {
    static displayName = ComposedComponent.displayName;

    static defaultProps = {
      onDraftReady: () => {},
    };

    static containerRequired = false;

    static getDerivedStateFromProps(
      props: InflatesDraftClientIdProps,
      state: InflatesDraftClientIdState
    ) {
      // If headerMessageId prop has changed, clear draft and session state
      // to prevent rendering with mismatched data
      if (props.headerMessageId !== state._lastHeaderMessageId) {
        return {
          draft: null,
          session: null,
          _lastHeaderMessageId: props.headerMessageId,
        };
      }
      return null;
    }

    _mounted = false;
    _sessionUnlisten?: () => void;

    constructor(props: Props) {
      super(props);
      this.state = {
        session: null,
        draft: null,
        _lastHeaderMessageId: props.headerMessageId,
      };
    }

    componentDidMount() {
      this._mounted = true;
      this._prepareForDraft(this.props.headerMessageId);
    }

    componentWillUnmount() {
      this._mounted = false;
      this._teardownForDraft();
      if (!this.props.keepPristineDraft) {
        this._deleteDraftIfEmpty();
      }
    }

    componentDidUpdate(prevProps: InflatesDraftClientIdProps) {
      if (prevProps.headerMessageId !== this.props.headerMessageId) {
        // getDerivedStateFromProps has already cleared draft/session state before render
        // Now we just need to teardown the old draft and prepare for the new one
        this._teardownForDraft();
        this._prepareForDraft(this.props.headerMessageId);
      }
    }

    async _prepareForDraft(headerMessageId: string) {
      if (!headerMessageId) {
        return;
      }
      const session = await DraftStore.sessionForClientId(headerMessageId);
      const shouldSetState = () =>
        this._mounted && session.headerMessageId === this.props.headerMessageId;

      if (!shouldSetState()) {
        return;
      }
      this._sessionUnlisten = session.listen(() => {
        if (!shouldSetState()) {
          return;
        }
        this.setState({ draft: session.draft() });
      });

      this.setState({
        session: session,
        draft: session.draft(),
      });
      this.props.onDraftReady?.();
    }

    _teardownForDraft() {
      if (this.state.session) {
        this.state.session.changes.commit();
      }
      if (this._sessionUnlisten) {
        this._sessionUnlisten();
      }
    }

    _deleteDraftIfEmpty() {
      if (!this.state.draft) {
        return;
      }
      if (this.state.draft.pristine) {
        Actions.destroyDraft(this.state.draft);
      }
    }

    // Returns a promise for use in composer/main.ts, to show the window
    // once the composer is rendered and focused.
    focus() {
      return Utils.waitFor(() => this.refs.composed)
        .then(() => (this.refs.composed as any).focus())
        .catch(() => {});
    }

    render() {
      const { draft, session } = this.state;
      // draft and session are set together
      if (!draft || !session) {
        return <span />;
      }
      return (
        // React strips key and ref from this.props, so their position doesn't matter at runtime
        <ComposedComponent
          {...this.props}
          {...this.state}
          draft={draft}
          session={session}
          key={draft.headerMessageId}
          ref="composed"
        />
      );
    }
  };
}

export default InflatesDraftClientId;
