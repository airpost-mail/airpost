import React from 'react';
import ReactDOM from 'react-dom';
import BaseStore from './base';
import * as Actions from '../actions';
import { UIProvider } from '../../components/kit/provider';
import FixedPopover from '../../components/fixed-popover';

const CONTAINER_ID = 'airpost-popover-container';

function createContainer(id: string) {
  const element = document.createElement(id);
  document.body.insertBefore(element, document.body.children[0]);
  return element;
}

class PopoverStore extends BaseStore {
  isOpen = false;
  container: HTMLElement;

  constructor(containerId = CONTAINER_ID) {
    super();
    this.container = createContainer(containerId);
    ReactDOM.render(
      <UIProvider>
        <span />
      </UIProvider>,
      this.container
    );

    this.listenTo(Actions.openPopover, this.openPopover);
    this.listenTo(Actions.closePopover, this.closePopover);
  }

  renderPopover = (
    child: React.ReactNode,
    props: React.ComponentProps<typeof FixedPopover>,
    callback: () => void
  ) => {
    const popover = (
      <UIProvider>
        <FixedPopover {...props}>{child}</FixedPopover>
      </UIProvider>
    );

    ReactDOM.render(popover, this.container, () => {
      this.isOpen = true;
      this.trigger();
      callback();
    });
  };

  openPopover = (
    element: React.ReactNode,
    {
      originRect,
      direction,
      fallbackDirection,
      closeOnAppBlur,
      callback = () => {},
    }: Omit<React.ComponentProps<typeof FixedPopover>, 'children'> & { callback?: () => void }
  ) => {
    const props = {
      direction,
      originRect,
      fallbackDirection,
      closeOnAppBlur,
    };

    if (this.isOpen) {
      this.closePopover(() => {
        this.renderPopover(element, props, callback);
      });
    } else {
      this.renderPopover(element, props, callback);
    }
  };

  closePopover = (callback = () => {}) => {
    const empty = (
      <UIProvider>
        <span />
      </UIProvider>
    );
    ReactDOM.render(empty, this.container, () => {
      this.isOpen = false;
      this.trigger();
      callback();
    });
  };
}

export default new PopoverStore();
