import { Message } from '../../data/models/message';
import { Thread } from '../../data/models/thread';

export interface ThreadWithMessagesMetadata extends Thread {
  __messages: Message[];
}
