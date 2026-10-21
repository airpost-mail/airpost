import React from 'react';
import { Account } from '../../data/models/account';
import { Contact } from '../../data/models/contact';
import { AccountStore } from '../../data/stores/account';
import { ContactGroup } from '../../data/models/contact-group';
import { ContactProfilePhoto } from '../../components/contact-profile-photo';
import { RetinaImg } from '../../components/retina-img';
import * as Icons from './svg-icons';
import { Button } from '../../components/kit/button';
import { Store } from './store';
import { ContactBase, ContactInteractorMetadata } from './types';

export const ContactDetailRead = ({
  data,
  groups,
  contact,
  metadata,
}: {
  data: ContactBase;
  groups: ContactGroup[];
  contact: Contact;
  metadata: ContactInteractorMetadata;
}) => {
  return (
    <div className="contact-detail-content-wrap">
      <div className="contact-hero">
        <ContactProfilePhoto contact={contact} loading={false} avatar={data.photoURL} />
        <h3>{data.name.displayName}</h3>
      </div>
      <div className="contact-group-memberships">
        {contact.contactGroups.map((gid) => {
          const group = groups.find((g) => g.id === gid);
          const label = group ? group.name : gid;
          return (
            <Button
              key={gid}
              variant="secondary"
              size="sm"
              className="group-membership"
              onClick={() => {
                Store.setPerspective({
                  label,
                  accountId: contact.accountId,
                  groupId: gid,
                  type: 'group',
                });
              }}
            >
              <RetinaImg name="label.png" mode={RetinaImg.Mode.ContentIsMask} />
              {label}
            </Button>
          );
        })}
      </div>
      {
        <ContactAttributes
          data={data}
          origin={metadata.origin}
          account={AccountStore.accountForId(contact.accountId)}
        />
      }
    </div>
  );
};

const ContactAttributes = ({
  data,
  origin,
  account,
}: {
  data: ContactBase;
  origin: string;
  account: Account | undefined;
}) => (
  <div className="contact-attributes">
    {data.nicknames && (
      <div className="contact-attributes-section">
        {data.nicknames.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true"></span>
            <div>{`”${item.value}”`}</div>
          </div>
        ))}
      </div>
    )}
    {(data.title || data.company) && (
      <div className="contact-attributes-section">
        <div className="contact-attribute">
          <span aria-hidden="true">
            <Icons.Briefcase />
          </span>
          <div>{`${data.title ? `${data.title}, ` : ''}${data.company}`}</div>
        </div>
      </div>
    )}
    {data.emailAddresses && (
      <div className="contact-attributes-section">
        {data.emailAddresses.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.Envelope />
            </span>
            <div>
              <a href={`mailto:${item.value}`} title="Send email...">
                {item.value}
              </a>
              {item.type && <div className="type">{item.type}</div>}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.phoneNumbers && (
      <div className="contact-attributes-section">
        {data.phoneNumbers.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.Phone />
            </span>
            <div>
              <a href={`tel:${item.value}`} title="Call...">
                {item.value}
              </a>
              {item.type && <div className="type">{item.type}</div>}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.addresses && (
      <div className="contact-attributes-section">
        {data.addresses.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.Map />
            </span>
            <div>
              <a href={`https://maps.google.com/?q=${encodeURIComponent(item.formattedValue)}`}>
                {item.formattedValue}
              </a>
              {item.type && <div className="type">{item.type}</div>}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.birthdays && (
      <div className="contact-attributes-section">
        {data.birthdays.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.Crown />
            </span>
            <div>
              {item.date
                ? item.date.year
                  ? new Date(
                      item.date.year,
                      (item.date.month ?? 0) - 1,
                      item.date.day ?? 0
                    ).toLocaleDateString()
                  : new Date(
                      new Date().getFullYear(),
                      (item.date.month ?? 0) - 1,
                      item.date.day ?? 0
                    ).toLocaleDateString(undefined, { month: 'numeric', day: 'numeric' })
                : ''}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.relations && (
      <div className="contact-attributes-section">
        {data.relations.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.People />
            </span>
            <div>
              {item.person}
              {item.type && <div className="type">{item.type}</div>}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.urls && (
      <div className="contact-attributes-section">
        {data.urls.map((item, idx) => (
          <div className="contact-attribute" key={idx}>
            <span aria-hidden="true">
              <Icons.Link />
            </span>
            <div>
              <a href={`${item.value}`} title="Visit website...">
                {item.value}
              </a>
              {item.type && <div className="type">{item.type}</div>}
            </div>
          </div>
        ))}
      </div>
    )}
    {data.notes && (
      <div className="contact-attributes-section">
        <div className="contact-attribute">
          <span aria-hidden="true">
            <Icons.Note />
          </span>
          <div style={{ whiteSpace: 'pre-wrap' }}>{data.notes}</div>
        </div>
      </div>
    )}
    <div className="contact-origin">
      <div>{`${origin} (${account ? account.label : 'Unknown Account'})`}</div>
    </div>
  </div>
);
