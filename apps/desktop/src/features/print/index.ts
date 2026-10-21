import Printer from './printer';

let printer: Printer | null = null;
export function activate() {
  printer = new Printer();
}

export function deactivate() {
  if (printer) printer.deactivate();
}

export function serialize() {}
