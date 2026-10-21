const Fields: {
  To: string;
  Cc: string;
  Bcc: string;
  From: string;
  Subject: string;
  Body: string;
  ReplyTo: string;
  ParticipantFields: string[];
  Order: { [field: string]: number };
} = {
  To: 'textFieldTo',
  Cc: 'textFieldCc',
  Bcc: 'textFieldBcc',
  From: 'fromField',
  Subject: 'textFieldSubject',
  Body: 'contentBody',
  ReplyTo: 'textFieldReplyTo',
  ParticipantFields: [],
  Order: {},
};

Fields.ParticipantFields = [Fields.To, Fields.Cc, Fields.Bcc];

Fields.Order = {
  textFieldTo: 1,
  textFieldCc: 2,
  textFieldBcc: 3,
  textFieldReplyTo: 4,
  fromField: -1, // Not selectable
  textFieldSubject: 5,
  contentBody: 6,
};

export default Fields;
