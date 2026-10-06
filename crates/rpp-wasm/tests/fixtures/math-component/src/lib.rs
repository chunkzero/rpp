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

    fn sleep_hour() -> u64 {
        let start = std::time::Instant::now();
        std::thread::sleep(std::time::Duration::from_secs(3600));
        start.elapsed().as_nanos() as u64
    }

    fn bytes(len: u32) -> Vec<u8> {
        vec![7; len as usize]
    }
}

export!(Component);
