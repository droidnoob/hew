// Sample TypeScript fixture for TS.4 end-to-end tests.
// Layout: two top-level functions, one class + two methods, one arrow fn.

export function alphaCompute(a: number, b: number): number {
  const scale = (x: number) => x * 2;
  return scale(a) + scale(b);
}

export function betaFormat(name: string): string {
  return `hello, ${name}`;
}

export class Widget {
  id: number;

  constructor(id: number) {
    this.id = id;
  }

  gammaDescribe(): string {
    return `widget-${this.id}`;
  }

  deltaClone(): Widget {
    return new Widget(this.id);
  }
}

export function epsilonDispatch(w: Widget): string {
  return w.gammaDescribe();
}
