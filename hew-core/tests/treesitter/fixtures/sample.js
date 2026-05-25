// Sample JavaScript fixture for TS.4 end-to-end tests.
// Layout: two top-level functions, one class + two methods, one arrow fn.

function alphaCompute(a, b) {
  const scale = (x) => x * 2;
  return scale(a) + scale(b);
}

function betaFormat(name) {
  return `hello, ${name}`;
}

class Widget {
  constructor(id) {
    this.id = id;
  }

  gammaDescribe() {
    return `widget-${this.id}`;
  }

  deltaClone() {
    return new Widget(this.id);
  }
}

function epsilonDispatch(w) {
  return w.gammaDescribe();
}
