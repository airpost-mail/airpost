import React from 'react';
import { Contact } from '../../data/models/contact';
import { ContactBase } from './types';
import { YYMMDDInput } from './yymmdd-input';
import { ListEditor } from './list-editor';
import { TypeaheadFreeInput } from './typeahead-free-input';
import * as Icons from './svg-icons';
import { ContactProfilePhoto } from '../../components/contact-profile-photo';
import { TextField, TextFieldProps } from '../../components/kit/text-field';
import { Textarea } from '../../components/kit/textarea';

let nextFieldId = 0;
const newFieldId = () => `contact-edit-field-${++nextFieldId}`;

// The label wraps the field, as the text-above-input layout expects; htmlFor ties them for
// tools that don't follow nesting into the kit component.
const LabeledTextField = ({ label, ...props }: TextFieldProps & { label: string }) => {
  const [id] = React.useState(newFieldId);
  return (
    <label htmlFor={id}>
      {label}
      <TextField id={id} {...props} />
    </label>
  );
};

const BaseTypes = ['Home', 'Work', 'Other'];

const PhoneTypes = [
  'Home',
  'Work',
  'Other',
  'Mobile',
  'Main',
  'Home Fax',
  'Work Fax',
  'Google Voice',
  'Pager',
];

const WebTypes = ['Profile', 'Blog', 'Home Page', 'Work'];

const RelationTypes = [
  'Spouse',
  'Child',
  'Mother',
  'Father',
  'Parent',
  'Brother',
  'Sister',
  'Friend',
  'Relative',
  'Manager',
  'Assistant',
  'Reference',
  'Partner',
  'Domestic Partner',
];

export class ContactDetailEdit extends React.Component<{
  data: ContactBase;
  contact: Contact;
  onChange: (changes: Partial<ContactBase>) => void;
}> {
  _notesId = newFieldId();

  render() {
    const { onChange, contact, data } = this.props;

    return (
      <div className="contact-detail-content-wrap">
        <div className="contact-edit-section">
          <div className="contact-edit-section-icon" style={{ padding: 0, marginTop: 0 }}>
            <ContactProfilePhoto contact={contact} loading={false} avatar={data.photoURL} />
          </div>
          <div className="contact-edit-section-content">
            <div className="contact-edit-field">
              <LabeledTextField
                label="First Name"
                value={data.name.givenName}
                onChange={(e) =>
                  onChange({ name: { ...data.name, givenName: e.currentTarget.value } })
                }
              />
            </div>
            <div className="contact-edit-field">
              <LabeledTextField
                label="Last Name"
                value={data.name.familyName}
                onChange={(e) =>
                  onChange({ name: { ...data.name, familyName: e.currentTarget.value } })
                }
              />
            </div>

            <ListEditor<ContactBase['nicknames'][0]>
              items={data.nicknames || []}
              itemTemplate={{ value: '' }}
              onChange={(items) => onChange({ nicknames: items })}
            >
              {(item, onChange) => (
                <div className="contact-edit-field">
                  <LabeledTextField
                    label="Nickname"
                    value={item.value}
                    onChange={(e) => onChange({ value: e.currentTarget.value })}
                  />
                </div>
              )}
            </ListEditor>
          </div>
        </div>

        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Briefcase />
          </div>
          <div className="contact-edit-section-content">
            <div className="contact-edit-twoup">
              <div className="contact-edit-field">
                <LabeledTextField
                  label="Title"
                  value={data.title}
                  onChange={(e) => onChange({ title: e.currentTarget.value })}
                />
              </div>
              <div className="contact-edit-field" style={{ flex: 0.7 }}>
                <LabeledTextField
                  label="Company"
                  value={data.company}
                  onChange={(e) => onChange({ company: e.currentTarget.value })}
                />
              </div>
            </div>
          </div>
        </div>

        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Envelope />
          </div>
          <div className="contact-edit-section-content">
            <ListEditor<ContactBase['emailAddresses'][0]>
              items={data.emailAddresses || []}
              itemTemplate={{ type: '', value: '' }}
              onChange={(items) => onChange({ emailAddresses: items })}
            >
              {(item, onChange) => (
                <div className="contact-edit-twoup">
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Email"
                      value={item.value}
                      onChange={(e) => onChange({ value: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field" style={{ flex: 0.7 }}>
                    <span aria-hidden="true" className="form-spacer" />
                    <TypeaheadFreeInput
                      placeholder="Label"
                      suggestions={BaseTypes}
                      value={item.type || ''}
                      onChange={(e) => onChange({ type: e.currentTarget.value })}
                    />
                  </div>
                </div>
              )}
            </ListEditor>
          </div>
        </div>

        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Phone />
          </div>
          <div className="contact-edit-section-content">
            <ListEditor<ContactBase['phoneNumbers'][0]>
              items={data.phoneNumbers || []}
              itemTemplate={{ type: '', value: '' }}
              onChange={(items) => onChange({ phoneNumbers: items })}
            >
              {(item, onChange) => (
                <div className="contact-edit-twoup">
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Phone"
                      value={item.value}
                      onChange={(e) => onChange({ value: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field" style={{ flex: 0.7 }}>
                    <span aria-hidden="true" className="form-spacer" />
                    <TypeaheadFreeInput
                      placeholder="Label"
                      suggestions={PhoneTypes}
                      value={item.type || ''}
                      onChange={(e) => onChange({ type: e.currentTarget.value })}
                    />
                  </div>
                </div>
              )}
            </ListEditor>
          </div>
        </div>
        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Map />
          </div>
          <div className="contact-edit-section-content">
            <ListEditor<ContactBase['addresses'][0]>
              items={data.addresses || []}
              itemTemplate={{
                type: '',
                formattedValue: '',
                city: '',
                country: '',
                postalCode: '',
                region: '',
                streetAddress: '',
                extendedAddress: '',
              }}
              onChange={(items) => onChange({ addresses: items })}
            >
              {(item, onChange) => (
                <div style={{ display: 'flex', flexDirection: 'column', flex: 1 }}>
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Street Address"
                      value={item.streetAddress}
                      onChange={(e) => onChange({ streetAddress: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Street Address line 2"
                      value={item.extendedAddress}
                      onChange={(e) => onChange({ extendedAddress: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="City"
                      value={item.city}
                      onChange={(e) => onChange({ city: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-twoup">
                    <div className="contact-edit-field">
                      <LabeledTextField
                        label="Region"
                        value={item.region}
                        onChange={(e) => onChange({ region: e.currentTarget.value })}
                      />
                    </div>
                    <div className="contact-edit-field" style={{ flex: 0.7 }}>
                      <LabeledTextField
                        label="Postal Code"
                        value={item.postalCode}
                        onChange={(e) => onChange({ postalCode: e.currentTarget.value })}
                      />
                    </div>
                  </div>

                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Country"
                      value={item.country}
                      onChange={(e) => onChange({ country: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field" style={{ flex: 0.7 }}>
                    <span aria-hidden="true" className="form-spacer" />
                    <TypeaheadFreeInput
                      aria-label="Type"
                      placeholder="Label"
                      suggestions={BaseTypes}
                      value={item.type || ''}
                      onChange={(e) => onChange({ type: e.currentTarget.value })}
                    />
                  </div>
                </div>
              )}
            </ListEditor>
          </div>
        </div>
        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Crown />
          </div>
          <div className="contact-edit-section-content">
            <ListEditor<ContactBase['birthdays'][0]>
              items={data.birthdays || []}
              itemTemplate={{ date: { year: null, month: null, day: null } }}
              onChange={(items) => onChange({ birthdays: items })}
            >
              {(item, onChange) => (
                <YYMMDDInput value={item.date} onChange={(date) => onChange({ date })} />
              )}
            </ListEditor>
          </div>
        </div>
        {data.relations !== undefined && (
          <div className="contact-edit-section">
            <div className="contact-edit-section-icon">
              <Icons.People />
            </div>
            <div className="contact-edit-section-content">
              <ListEditor<ContactBase['relations'][0]>
                items={data.relations || []}
                itemTemplate={{ person: '', type: '' }}
                onChange={(items) => onChange({ relations: items })}
              >
                {(item, onChange) => (
                  <div className="contact-edit-twoup">
                    <div className="contact-edit-field">
                      <LabeledTextField
                        label="Relation"
                        value={item.person}
                        onChange={(e) => onChange({ person: e.currentTarget.value })}
                      />
                    </div>
                    <div className="contact-edit-field" style={{ flex: 0.7 }}>
                      <span aria-hidden="true" className="form-spacer" />
                      <TypeaheadFreeInput
                        placeholder="Label"
                        suggestions={RelationTypes}
                        value={item.type || ''}
                        onChange={(e) => onChange({ type: e.currentTarget.value })}
                      />
                    </div>
                  </div>
                )}
              </ListEditor>
            </div>
          </div>
        )}
        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Link />
          </div>
          <div className="contact-edit-section-content">
            <ListEditor<ContactBase['urls'][0]>
              items={data.urls || []}
              itemTemplate={{ value: '', type: '' }}
              onChange={(items) => onChange({ urls: items })}
            >
              {(item, onChange) => (
                <div className="contact-edit-twoup">
                  <div className="contact-edit-field">
                    <LabeledTextField
                      label="Link"
                      value={item.value}
                      onChange={(e) => onChange({ value: e.currentTarget.value })}
                    />
                  </div>
                  <div className="contact-edit-field" style={{ flex: 0.7 }}>
                    <span aria-hidden="true" className="form-spacer" />
                    <TypeaheadFreeInput
                      placeholder="Label"
                      suggestions={WebTypes}
                      value={item.type || ''}
                      onChange={(e) => onChange({ type: e.currentTarget.value })}
                    />
                  </div>
                </div>
              )}
            </ListEditor>
          </div>
        </div>
        <div className="contact-edit-section">
          <div className="contact-edit-section-icon">
            <Icons.Note />
          </div>
          <div className="contact-edit-section-content">
            <div className="contact-edit-field">
              <label htmlFor={this._notesId}>
                Notes
                <Textarea
                  id={this._notesId}
                  className="contact-notes-textarea"
                  value={data.notes || ''}
                  onChange={(e) => onChange({ notes: e.currentTarget.value })}
                  rows={4}
                />
              </label>
            </div>
          </div>
        </div>
      </div>
    );
  }
}
