// Sample Rust fixture for TS.4 end-to-end tests.
// Layout: two top-level fns, one struct + two methods, one closure.

pub fn alpha_compute(a: i32, b: i32) -> i32 {
    let multiplier = 2;
    let scale = |x: i32| x * multiplier;
    scale(a) + scale(b)
}

pub fn beta_format(name: &str) -> String {
    format!("hello, {name}")
}

pub struct Widget {
    pub id: u32,
}

impl Widget {
    pub fn gamma_describe(&self) -> String {
        format!("widget-{}", self.id)
    }

    pub fn delta_clone(&self) -> Self {
        Widget { id: self.id }
    }
}

pub fn epsilon_dispatch(w: &Widget) -> String {
    w.gamma_describe()
}
