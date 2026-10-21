type ServiceCallback = (service: unknown) => void;

// Services are whatever their plugin registers; `T` is the caller's word for what `name` is.
class ServiceRegistry {
  private _waitingForServices: { [name: string]: ServiceCallback[] } = {};
  private _services: { [name: string]: unknown } = {};

  withService<T = unknown>(name: string, callback: (service: T) => void) {
    const call = callback as ServiceCallback;
    if (this._services[name]) {
      setTimeout(() => call(this._services[name]), 0);
    } else {
      this._waitingForServices[name] = this._waitingForServices[name] || [];
      this._waitingForServices[name].push(call);
    }
  }

  registerService(name: string, obj: unknown) {
    this._services[name] = obj;
    if (this._waitingForServices[name]) {
      for (const callback of this._waitingForServices[name]) {
        callback(obj);
      }
      delete this._waitingForServices[name];
    }
  }

  unregisterService(name: string) {
    delete this._services[name];
  }
}

export default new ServiceRegistry();
