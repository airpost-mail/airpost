import { EventEmitter } from 'events';

// A very, very simple Flux implementation

export type ListenableCallback = (...args: any[]) => void;

export interface Listenable {
  listen: (callback: ListenableCallback, thisArg: BaseStore) => () => void;
  hasListener?: (a: any) => boolean;
}

export default class BaseStore implements Listenable {
  // Both are made on first use.
  _emitter?: EventEmitter;
  subscriptions?: Array<{ stop: () => void; listenable: Listenable }>;

  hasListener(listenable: Listenable) {
    for (const sub of this.subscriptions || []) {
      if (
        sub.listenable === listenable ||
        (sub.listenable.hasListener && sub.listenable.hasListener(listenable))
      ) {
        return true;
      }
    }
    return false;
  }

  validateListening(listenable: Listenable) {
    if (listenable === this) {
      return 'Listener is not able to listen to itself';
    }
    if (!(listenable.listen instanceof Function)) {
      console.log(require('util').inspect(listenable));
      console.log(new Error().stack);
      return listenable + ' is missing a listen method';
    }
    if (listenable.hasListener && listenable.hasListener(this)) {
      return 'Listener cannot listen to this listenable because of circular loop';
    }
  }

  listenTo(listenable: Listenable, callback: ListenableCallback) {
    const subscriptions = (this.subscriptions = this.subscriptions || []);

    const err = this.validateListening(listenable);
    if (err) {
      throw err;
    }

    if (!callback) {
      throw new Error('@listenTo called with no callback');
    }
    const desub = listenable.listen(callback, this);

    const subscription = {
      stop: () => {
        const index = subscriptions.indexOf(subscription);
        if (index === -1) {
          throw new Error('Tried to remove listen already gone from subscriptions list!');
        }
        subscriptions.splice(index, 1);
        desub();
      },
      listenable,
    };
    subscriptions.push(subscription);

    return subscription;
  }

  stopListeningTo(listenable: Listenable) {
    const subs = this.subscriptions || [];
    for (const sub of subs) {
      if (sub.listenable === listenable) {
        sub.stop();
        if (subs.indexOf(sub) !== -1) {
          throw new Error('Failed to remove listen from subscriptions list!');
        }
        return true;
      }
    }
    return false;
  }

  stopListeningToAll() {
    let remaining = undefined;
    const subs = this.subscriptions || [];
    while ((remaining = subs.length)) {
      subs[0].stop();
      if (subs.length !== remaining - 1) {
        throw new Error('Failed to remove listen from subscriptions list!');
      }
    }
  }

  // Returns the emitter, making it on first use.
  setupEmitter(): EventEmitter {
    if (this._emitter) {
      return this._emitter;
    }
    this._emitter = new EventEmitter();
    return this._emitter.setMaxListeners(250);
  }

  listen(callback: ListenableCallback, bindContext: any = this) {
    if (!callback) {
      throw new Error('@listen called with undefined callback');
    }

    const emitter = this.setupEmitter();

    let aborted = false;

    const eventHandler = (...args: unknown[]) => {
      if (aborted) {
        return;
      }
      return callback.apply(bindContext, args);
    };

    emitter.addListener('trigger', eventHandler);

    return () => {
      aborted = true;
      return emitter.removeListener('trigger', eventHandler);
    };
  }

  trigger(...args: unknown[]) {
    return this.setupEmitter().emit('trigger', ...args);
  }
}
