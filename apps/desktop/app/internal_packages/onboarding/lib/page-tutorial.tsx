import { localized, React } from 'mailspring-exports';
import * as OnboardingActions from './onboarding-actions';

const Features = [
  {
    id: 'people',
    title: localized('Compose with context'),
    description: localized(
      'Tern shows you everything about your contacts right inside your inbox. See LinkedIn profiles, Twitter bios, message history, and more.'
    ),
  },
  {
    id: 'activity',
    title: localized('Track opens and clicks'),
    description: localized(
      'With activity tracking, you’ll know as soon as someone reads your message. Sending to a group? Tern shows you which recipients opened your email so you can follow up with precision.'
    ),
  },
  {
    id: 'snooze',
    title: localized('Send on your own schedule'),
    description: localized(
      'Snooze emails to return at any time that suits you. Schedule messages to send at the ideal time. Tern makes it easy to control the fabric of spacetime!'
    ),
  },
];

export default class TutorialPage extends React.Component<
  Record<string, unknown>,
  { appeared: boolean }
> {
  static displayName = 'TutorialPage';

  _timer: NodeJS.Timeout;

  state = { appeared: false };

  componentDidMount() {
    this._timer = setTimeout(() => {
      this.setState({ appeared: true });
    }, 200);
  }

  componentWillUnmount() {
    clearTimeout(this._timer);
  }

  render() {
    return (
      <div className={`page tutorial appeared-${this.state.appeared}`}>
        <div className="tutorial-container">
          {Features.map((feature) => (
            <div key={feature.id} className="feature">
              <h2>{feature.title}</h2>
              <p>{feature.description}</p>
            </div>
          ))}
        </div>
        <div className="footer">
          <button
            key="prev"
            className="btn btn-large btn-prev"
            onClick={() => OnboardingActions.moveToPreviousPage()}
          >
            {localized('Back')}
          </button>
          <button
            key="next"
            className="btn btn-large btn-next"
            onClick={() => OnboardingActions.moveToPage('account-choose')}
          >
            {localized('Get Started')}
          </button>
        </div>
      </div>
    );
  }
}
