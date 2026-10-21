import React from 'react';
import ReactDOM from 'react-dom';
import Modal from '../../components/modal';
import BaseStore from './base';
import { UIProvider } from '../../components/kit/provider';

import * as Actions from '../actions';

const CONTAINER_ID = 'tern-modal-container';

function createContainer(id: string) {
  const element = document.createElement(id);
  document.body.insertBefore(element, document.body.children[0]);
  return element;
}

class ModalStore extends BaseStore {
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

    this.listenTo(Actions.openModal, this.openModal);
    this.listenTo(Actions.closeModal, this.closeModal);
  }

  isModalOpen = () => {
    return this.isOpen;
  };

  renderModal = (
    child: React.ReactNode,
    props: React.ComponentProps<typeof Modal>,
    callback: () => void
  ) => {
    const modal = (
      <UIProvider>
        <Modal {...props}>{child}</Modal>
      </UIProvider>
    );

    ReactDOM.render(modal, this.container, () => {
      this.isOpen = true;
      this.trigger();
      callback();
    });
  };

  openModal = (
    { component, height, width }: { component: React.ReactNode; height?: number; width?: number },
    callback = () => {}
  ) => {
    const props = {
      height: height,
      width: width,
    };

    if (this.isOpen) {
      this.closeModal(() => {
        this.renderModal(component, props, callback);
      });
    } else {
      this.renderModal(component, props, callback);
    }
  };

  closeModal = (callback = () => {}) => {
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

export default new ModalStore();
