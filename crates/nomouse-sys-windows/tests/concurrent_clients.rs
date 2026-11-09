//! UI Automation clients created at the same moment on several threads (as parallel tests do)
//! must all come up: the client and its cache request are not created concurrently.
#![cfg(windows)]

use std::sync::{Arc, Barrier};
use std::thread;

use nomouse_sys_windows::Automation;

#[test]
fn clients_created_concurrently_all_start() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 10;
    for round in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(THREADS));
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    Automation::new().map(|_| ())
                })
            })
            .collect();
        for worker in workers {
            let result = worker.join().expect("worker thread");
            assert_eq!(result, Ok(()), "round {round}");
        }
    }
}
