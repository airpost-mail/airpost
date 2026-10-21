import { Task } from './task';
import * as Attributes from '../attributes';
import { localized } from '../../intl/localize';
import { AttributeValues } from '../models/model';

export class ChangeRoleMappingTask extends Task {
  static attributes = {
    ...Task.attributes,

    path: Attributes.String({
      modelKey: 'path',
    }),
    role: Attributes.String({
      modelKey: 'role',
    }),
  };

  declare path: string;
  declare role: string;

  constructor(data: AttributeValues<typeof ChangeRoleMappingTask.attributes> = {}) {
    super(data);
  }

  label() {
    return localized(`Changing folder mapping...`);
  }
}
