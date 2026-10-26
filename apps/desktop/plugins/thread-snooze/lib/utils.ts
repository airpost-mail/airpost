import moment, { Moment } from 'moment';
import { localized, DateUtils } from 'mailspring-exports';

export function snoozedUntilMessage(snoozeDate: string | null, now: Moment = moment()) {
  let message = localized('Snoozed');
  if (snoozeDate) {
    const dateFormat = DateUtils.DATE_FORMAT_SHORT;
    const date = moment(snoozeDate);
    const hourDifference = moment.duration(date.diff(now)).asHours();

    message += ` ${DateUtils.format(date, dateFormat)}`;
  }
  return message;
}
