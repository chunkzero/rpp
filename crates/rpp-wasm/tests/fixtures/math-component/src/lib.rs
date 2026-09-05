wit_bindgen::generate!({
    world: "math",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    fn spin() {
        loop {
            core::hint::spin_loop();
        }
    }
}

export!(Component);
