import url from 'url';
import querystring from 'querystring';
import { ipcRenderer } from 'electron';
import { localized } from '../../intl/localize';
import DatabaseStore from '../../data/database/store';
import { Thread } from '../../data/models/thread';
import { Matcher } from '../../data/attributes/matcher';
import * as Actions from '../../data/actions';

const DATE_EPSILON = 60; // Seconds

interface PermalinkParams {
  subject: string;
  lastDate?: number;
  date?: number;
}

const _parseOpenThreadUrl = (urlString: string) => {
  const parsedUrl = url.parse(urlString);
  const params = querystring.parse(parsedUrl.query ?? '') as any;
  return {
    subject: params.subject,
    date: params.date ? parseInt(params.date, 10) : undefined,
    lastDate: params.lastDate ? parseInt(params.lastDate, 10) : undefined,
  } as PermalinkParams;
};

const _findCorrespondingThread = (
  { subject, lastDate, date }: PermalinkParams,
  dateEpsilon = DATE_EPSILON
) => {
  const fromUnix = (seconds: number) => new Date(seconds * 1000);
  // A link without dates gives invalid dates, which match no thread.
  const last = lastDate ?? NaN;
  const dateClause = date
    ? new Matcher.And([
        Thread.attributes.firstMessageTimestamp.lessThan(fromUnix(date + dateEpsilon)),
        Thread.attributes.firstMessageTimestamp.greaterThan(fromUnix(date - dateEpsilon)),
      ])
    : new Matcher.Or([
        new Matcher.And([
          Thread.attributes.lastMessageSentTimestamp.lessThan(fromUnix(last + dateEpsilon)),
          Thread.attributes.lastMessageSentTimestamp.greaterThan(fromUnix(last - dateEpsilon)),
        ]),
        new Matcher.And([
          Thread.attributes.lastMessageReceivedTimestamp.lessThan(fromUnix(last + dateEpsilon)),
          Thread.attributes.lastMessageReceivedTimestamp.greaterThan(fromUnix(last - dateEpsilon)),
        ]),
      ]);

  return DatabaseStore.findBy<Thread>(Thread).where([
    Thread.attributes.subject.equal(subject),
    dateClause,
  ]);
};

const _onOpenThreadFromWeb = (event: Electron.IpcRendererEvent, permalink: string) => {
  const params = _parseOpenThreadUrl(permalink);

  _findCorrespondingThread(params)
    .then((thread) => {
      if (!thread) {
        throw new Error('Thread not found');
      }
      Actions.popoutThread(thread);
    })
    .catch((error) => {
      AppEnv.reportError(error);
      AppEnv.showErrorDialog(
        localized(`The thread %@ does not exist in your mailbox!`, params.subject)
      );
    });
};

export function activate() {
  ipcRenderer.on('openThreadFromWeb', _onOpenThreadFromWeb);
}
