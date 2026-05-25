# Sample Python fixture for TS.4 end-to-end tests.
# Layout: two top-level defs, one class + two methods, one lambda.


def alpha_compute(a, b):
    scale = lambda x: x * 2
    return scale(a) + scale(b)


def beta_format(name):
    return f"hello, {name}"


class Widget:
    def __init__(self, ident):
        self.id = ident

    def gamma_describe(self):
        return f"widget-{self.id}"

    def delta_clone(self):
        return Widget(self.id)


def epsilon_dispatch(w):
    return w.gamma_describe()
