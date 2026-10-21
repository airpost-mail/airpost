/**
 * Public: This keeps track of constructors so we know how to inflate
 * serialized objects.
 *
 * We map constructor string names with factory functions that will return
 * the actual constructor itself.
 *
 * The reason we have an extra function call to return a constructor is so
 * we don't need to `require` all constructors at once on load. We are
 * wasting a very large amount of time on bootup requiring files that may
 * never be used or only used way down the line.
 *
 * If 3rd party packages want to register new inflatable models, they can
 * use `register` and pass the constructor generator along with the name.
 *
 * Note that there is one registry per window.
 */
type ConstructorFactory = () => new (...args: unknown[]) => unknown;

/** What `deserialize` builds: an object that inflates itself from JSON. */
export interface Deserializable {
  fromJSON(json: unknown): unknown;
}

export default class SerializableRegistry<T extends Deserializable = Deserializable> {
  _constructorFactories: { [name: string]: ConstructorFactory } = {};

  get(name: string) {
    return this._constructorFactories[name].call(null);
  }

  getAllConstructors() {
    const constructors: ReturnType<ConstructorFactory>[] = [];
    for (const name of Object.keys(this._constructorFactories)) {
      constructors.push(this.get(name));
    }
    return constructors;
  }

  isInRegistry(name: string) {
    return !!this._constructorFactories[name];
  }

  // U lets a caller name the registered subclass it expects back.
  deserialize<U extends T = T>(name: string, dataJSON: string | object): U {
    let data = dataJSON;
    if (typeof data === 'string') {
      data = JSON.parse(data);
    }

    const constructor = this.get(name);

    if (typeof constructor !== 'function') {
      throw new Error(`Unsure of how to inflate ${JSON.stringify(data)}. \
Your constructor factory must return a class constructor.`);
    }

    // Whoever registers `name` registers a class whose instances are T.
    const object = new constructor() as U;
    object.fromJSON(data);

    return object;
  }

  register(name: string, constructorFactory: ConstructorFactory) {
    this._constructorFactories[name] = constructorFactory;
  }

  unregister(name: string) {
    delete this._constructorFactories[name];
  }
}
