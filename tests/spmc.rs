#![allow(clippy::missing_panics_doc, reason = "It's a test")]

use ringbeam::Error;

#[cfg(feature = "loom")]
mod thread {
    pub use loom::thread::{spawn, yield_now};
}
#[cfg(not(feature = "loom"))]
mod thread {
    pub use std::thread::{spawn, yield_now};
}
#[cfg(feature = "loom")]
use loom::model::model;
#[cfg(not(feature = "loom"))]
fn model<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    f();
}

#[test]
pub fn test_spmc_try_send_recv_sequential() {
    model(|| {
        let (sender, receiver) = ringbeam::spmc::bounded::<64, u8>();
        sender.try_send(10).unwrap();
        let res = receiver.try_recv().unwrap();
        assert_eq!(res, 10);
    });
}

#[test]
pub fn test_spmc_try_send_recv_interleaved_1() {
    model(|| {
        let (sender, receiver) = ringbeam::spmc::bounded::<64, u8>();
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
pub fn test_spmc_try_send_recv_interleaved_2() {
    model(|| {
        let (send_result, recv_result) = std::sync::mpsc::channel::<usize>();
        let (sender, receiver) = ringbeam::spmc::bounded::<64, u8>();
        let send_result_1 = send_result.clone();
        let receiver_1 = receiver.clone();
        let handle = thread::spawn(move || {
            let mut total = 0usize;
            loop {
                match receiver_1.try_recv() {
                    Ok(val) => {
                        total += usize::from(val);
                    }
                    Err(Error::Empty) => thread::yield_now(),
                    Err(Error::Closed) => break,
                    Err(err) => panic!("{err:?}"),
                }
            }
            send_result_1.send(total).unwrap();
        });
        let handle2 = thread::spawn(move || {
            let mut total = 0usize;
            loop {
                match receiver.try_recv() {
                    Ok(val) => {
                        total += usize::from(val);
                    }
                    Err(Error::Empty) => thread::yield_now(),
                    Err(Error::Closed) => break,
                    Err(err) => panic!("{err:?}"),
                }
            }
            send_result.send(total).unwrap();
        });
        let handle3 = thread::spawn(move || {
            for i in 0..100u8 {
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
        handle3.join().unwrap();
        let got = recv_result.iter().reduce(|acc, x| acc + x).unwrap();
        let expected = (0..100usize).reduce(|acc, x| acc + x).unwrap();
        assert_eq!(got, expected);
    });
}
