#![allow(clippy::missing_panics_doc, missing_docs, reason = "It's a test")]

use ringbeam::Error;

#[cfg(feature = "_loom")]
mod thread {
    pub use loom::thread::{spawn, yield_now};
}
#[cfg(not(feature = "_loom"))]
mod thread {
    pub use std::thread::{spawn, yield_now};
}
#[cfg(feature = "_loom")]
use loom::model::model;
#[cfg(not(feature = "_loom"))]
fn model<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    f();
}

#[test]
pub fn test_mpsc_try_send_recv_sequential() {
    model(|| {
        let (sender, receiver) = ringbeam::mpsc::bounded::<64, u8>();
        sender.try_send(10).unwrap();
        let res = receiver.try_recv().unwrap();
        assert_eq!(res, 10);
    });
}

#[test]
pub fn test_mpsc_try_send_recv_interleaved_1() {
    model(|| {
        let (sender, receiver) = ringbeam::mpsc::bounded::<64, u8>();
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

#[test]
pub fn test_mpsc_try_send_recv_interleaved_2() {
    model(|| {
        let (sender, receiver) = ringbeam::mpsc::bounded::<64, u8>();
        let handle = thread::spawn(move || {
            let mut i = 0;
            let mut j = 1;
            loop {
                match receiver.try_recv() {
                    Ok(val) => {
                        if val.is_multiple_of(2) {
                            assert_eq!(i, val);
                            i += 2;
                        } else {
                            assert_eq!(j, val);
                            j += 2;
                        }
                    }
                    Err(Error::Empty) => thread::yield_now(),
                    Err(err) => panic!("{err:?}"),
                }
                if i == 100 && j == 101 {
                    break;
                }
            }
        });
        let sender2 = sender.clone();
        let handle2 = thread::spawn(move || {
            for i in 0..100u8 {
                if i.is_multiple_of(2) {
                    loop {
                        match sender2.try_send(i) {
                            Ok(None) => break,
                            Ok(_) => thread::yield_now(),
                            Err(err) => panic!("{err:?}"),
                        }
                    }
                }
            }
        });
        let handle3 = thread::spawn(move || {
            for i in 0..100u8 {
                if !i.is_multiple_of(2) {
                    loop {
                        match sender.try_send(i) {
                            Ok(None) => break,
                            Ok(_) => thread::yield_now(),
                            Err(err) => panic!("{err:?}"),
                        }
                    }
                }
            }
        });
        handle.join().unwrap();
        handle2.join().unwrap();
        handle3.join().unwrap();
    });
}
