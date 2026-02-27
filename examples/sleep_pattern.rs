use std::{
    thread::sleep,
    time::{Duration, Instant},
};

#[inline(never)]
pub fn do_work() {
    let duration = Duration::from_millis(200); // Lock for 5 seconds
    let start = Instant::now();

    // Spin lock: Keep the CPU core busy
    while start.elapsed() < duration {
        // Perform intensive, non-blocking calculations here
        // Do NOT use thread::sleep here, or you will release the CPU
    }
}

fn main() {
    loop {
        do_work();
        sleep(Duration::from_millis(1000));
    }
}
