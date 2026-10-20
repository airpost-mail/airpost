import { Message, MessageViewExtension } from 'mailspring-exports';
import AutoloadImagesStore from './store';

export default class AutoloadImagesExtension extends MessageViewExtension {
  static allowsRemoteImages = ({ message }: { message: Message }) =>
    AutoloadImagesStore.remoteImagesAllowed(message);
}
