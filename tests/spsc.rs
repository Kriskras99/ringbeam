#![allow(clippy::missing_panics_doc, reason = "It's a test")]

use ringbeam::Error;

#[cfg(loom)]
mod thread {
    pub use loom::thread::{spawn, yield_now};
}
#[cfg(not(loom))]
mod thread {
    pub use std::thread::{spawn, yield_now};
}
#[cfg(loom)]
use loom::model::model;
#[cfg(not(loom))]
fn model<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    f();
}

#[test]
pub fn test_spsc_try_send_recv_sequential() {
    model(|| {
        let (sender, receiver) = ringbeam::spsc::<64, u8>();
        sender.try_send(10).unwrap();
        let res = receiver.try_recv().unwrap();
        assert_eq!(res, 10);
    });
}

#[test]
pub fn test_spsc_try_send_recv_interleaved() {
    model(|| {
        let (sender, receiver) = ringbeam::spsc::<64, u8>();
        let handle = thread::spawn(move || {
            for i in 0..100 {
                loop {
                    match receiver.try_recv() {
                        Ok(val) => {
                            assert_eq!(val, i);
                            break;
                        }
                        Err(Error::Empty) => thread::yield_now(),
                        Err(err) => panic!("{err:?}"),
                    }
                }
            }
        });
        let handle2 = thread::spawn(move || {
            for i in 0..100 {
                loop {
                    match sender.try_send(i) {
                        Ok(None) => break,
                        Ok(_) => thread::yield_now(),
                        Err(err) => panic!("{err:?}"),
                    }
                }
            }
        });
        handle.join().unwrap();
        handle2.join().unwrap();
    });
}
