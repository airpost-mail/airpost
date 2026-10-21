import { Message } from '../../data/models/message';
import { MessageViewExtension } from '../../packages/message-view-extension';
import AutoloadImagesStore from './store';

export default class AutoloadImagesExtension extends MessageViewExtension {
  static allowsRemoteImages = ({ message }: { message: Message }) =>
    AutoloadImagesStore.remoteImagesAllowed(message);
}
