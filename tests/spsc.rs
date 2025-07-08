#[test]
pub fn test_spsc_try_send_recv() {
    let (sender, receiver) = ringbeam::spsc::<63, u8>();
    sender.try_send(10).unwrap();
    let res = receiver.try_recv().unwrap();
    assert_eq!(res, 10);
}
