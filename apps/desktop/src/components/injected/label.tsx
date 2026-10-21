/* eslint react/prefer-stateless-function: 0 */
import React from 'react';
import { ComponentRegistryDescriptor } from '../../packages/component-registry';

type LocationDescriptor = NonNullable<ComponentRegistryDescriptor['location']>;

// Toolbar declarations ({Left, Right}) have no id; they print as undefined, as before.
const locationId = (loc: LocationDescriptor | undefined) =>
  loc && 'id' in loc ? loc.id : undefined;

type InjectedComponentLabelProps = {
  matching?: ComponentRegistryDescriptor;
};

export default class InjectedComponentLabel extends React.Component<InjectedComponentLabelProps> {
  static displayName = 'InjectedComponentLabel';

  render() {
    const matchingDescriptions: string[] = [];
    const matching = this.props.matching || {};

    for (const key of Object.keys(matching) as Array<keyof ComponentRegistryDescriptor>) {
      let val: unknown = matching[key];
      if (key === 'location') {
        val = locationId(matching.location);
      }
      if (key === 'locations') {
        val = matching.locations?.map(locationId);
      }
      matchingDescriptions.push(`${key}: ${val}`);
    }

    const propDescriptions: string[] = [];
    for (const [key, val] of Object.entries(this.props)) {
      if (key === 'matching') {
        continue;
      }
      const desc = val && val.constructor ? val.constructor.name : typeof val;
      propDescriptions.push(`${key}:<${desc}>`);
    }

    let description = ` ${matchingDescriptions.join(', ')}`;
    if (propDescriptions.length > 0) {
      description += ` (${propDescriptions.join(', ')})`;
    }

    return <span className="name">{description}</span>;
  }
}
