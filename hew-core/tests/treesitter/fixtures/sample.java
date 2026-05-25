// Sample Java fixture for TS.4 end-to-end tests.
// Layout: one top-level class with two static fns + two instance methods.
// Java requires everything inside a class — "top-level functions" surface
// as static methods on the outer class.

class Sample {
    public static int alphaCompute(int a, int b) {
        return a * 2 + b * 2;
    }

    public static String betaFormat(String name) {
        return "hello, " + name;
    }

    static class Widget {
        int id;

        Widget(int id) {
            this.id = id;
        }

        public String gammaDescribe() {
            return "widget-" + this.id;
        }

        public Widget deltaClone() {
            return new Widget(this.id);
        }
    }

    public static String epsilonDispatch(Widget w) {
        return w.gammaDescribe();
    }
}
