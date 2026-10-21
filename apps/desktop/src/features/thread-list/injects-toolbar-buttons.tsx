import React, { Component } from 'react';
import { Thread } from '../../data/models/thread';
import ListensToObservable from '../../components/decorators/listens-to-observable';
import { InjectedComponentSet } from '../../components/injected/component-set';
import ThreadListStore from './store';
import { ListSelection } from '../../components/list/selection';

export const ToolbarRole = 'ThreadActionsToolbarButton';

function defaultObservable() {
  return ThreadListStore.selectionObservable();
}

export interface InjectedToolbarProps {
  items: Thread[];
  selection: ListSelection;
  injectedButtons: React.ReactElement;
}

function InjectsToolbarButtons(
  ToolbarComponent: React.ComponentType<InjectedToolbarProps> & { displayName?: string },
  {
    getObservable,
    extraRoles = [],
  }: { getObservable?: () => Rx.Observable<any>; extraRoles: string[] }
) {
  const roles = [ToolbarRole].concat(extraRoles);

  class ComposedComponent extends Component<{ items: any[] }> {
    static displayName = ToolbarComponent.displayName;

    static containerRequired = false;

    render() {
      const { items } = this.props;
      const { selection } = ThreadListStore.dataSource();

      // Keep all of the exposed props from deprecated regions that now map to this one
      const exposedProps = {
        items,
        selection,
        thread: items[0],
      };
      const injectedButtons = (
        <InjectedComponentSet key="injected" matching={{ roles }} exposedProps={exposedProps} />
      );
      return (
        <ToolbarComponent items={items} selection={selection} injectedButtons={injectedButtons} />
      );
    }
  }

  const getStateFromObservable = (items: Thread[] | null) => {
    if (!items) {
      return { items: [] };
    }
    return { items };
  };
  return ListensToObservable(ComposedComponent, {
    getObservable: getObservable || defaultObservable,
    getStateFromObservable,
  });
}

export default InjectsToolbarButtons;
