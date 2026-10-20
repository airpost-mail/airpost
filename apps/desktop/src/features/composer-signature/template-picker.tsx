import React from 'react';
import { localized } from 'mailspring-exports';
import { RadioGroup, RadioGroupItem } from '../../components/kit/radio-group';
import Templates from './templates';

// The Raw HTML choice has no template name; Radix needs a non-empty value for it.
const RAW_HTML = '__raw__';

export default class SignatureTemplatePicker extends React.Component<{
  resolvedData: any;
  onChange: (e: { target: { id: string; value: string } }) => void;
}> {
  _el: HTMLElement;

  _onValueChange = (value: string) => {
    this.props.onChange({
      target: { id: 'templateName', value: value === RAW_HTML ? undefined : value },
    });
  };

  componentDidMount() {
    this.ensureSelectionVisible();
  }

  componentDidUpdate() {
    this.ensureSelectionVisible();
  }

  ensureSelectionVisible() {
    const item = this._el.querySelector('.active') as HTMLElement;
    if (item) {
      const left = item.offsetLeft - 5;
      const right = item.offsetLeft + item.clientWidth + 5;

      if (left < this._el.scrollLeft) {
        this._el.scrollLeft = left;
      } else if (right > this._el.scrollLeft + this._el.clientWidth) {
        this._el.scrollLeft = right - this._el.clientWidth;
      }
    }
  }

  render() {
    let { resolvedData } = this.props;

    const { name, email, title } = resolvedData;
    if (!name && !email && !title) {
      resolvedData = {
        ...resolvedData,
        name: localized('Your name'),
        email: 'you@domain.com',
        title: localized('Job Title'),
      };
    }
    const selected = resolvedData.templateName || RAW_HTML;
    return (
      <RadioGroup
        ref={(el) => (this._el = el)}
        className="signature-template-picker"
        orientation="horizontal"
        aria-label={localized('Style')}
        value={selected}
        onValueChange={this._onValueChange}
      >
        {Templates.map((t, idx) => (
          <RadioGroupItem
            key={idx}
            value={t.name}
            className={`option ${t.name === selected ? 'active' : ''}`}
          >
            <div className="centered">
              <div className="preview">{t(resolvedData)}</div>
            </div>
          </RadioGroupItem>
        ))}
        <RadioGroupItem
          value={RAW_HTML}
          className={`option ${selected === RAW_HTML ? 'active' : ''}`}
        >
          <div className="centered">{localized('Raw HTML')}</div>
        </RadioGroupItem>
      </RadioGroup>
    );
  }
}
