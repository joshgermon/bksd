use bksd::adapters::SimulatedAdapter;
use bksd::core::{HardwareAdapter, HardwareEvent};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

#[tokio::test]
async fn test_add_device() {
    let (adapter, controller) = SimulatedAdapter::new();
    let (tx, mut rx) = mpsc::channel(32);

    adapter.start(tx);

    controller.add_device("test-uuid-1", 64);

    let event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");

    match event {
        HardwareEvent::DeviceAdded(device) => {
            assert_eq!(device.uuid, "test-uuid-1");
            assert_eq!(device.capacity, 64 * 1024 * 1024 * 1024);
            assert!(device.label.contains("test-uuid-1"));
        }
        _ => panic!("expected DeviceAdded event"),
    }
}

#[tokio::test]
async fn test_remove_device() {
    let (adapter, controller) = SimulatedAdapter::new();
    let (tx, mut rx) = mpsc::channel(32);

    adapter.start(tx);

    controller.remove_device("test-uuid-2");

    let event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");

    match event {
        HardwareEvent::DeviceRemoved(uuid) => {
            assert_eq!(uuid, "test-uuid-2");
        }
        _ => panic!("expected DeviceRemoved event"),
    }
}

#[tokio::test]
async fn test_multiple_events() {
    let (adapter, controller) = SimulatedAdapter::new();
    let (tx, mut rx) = mpsc::channel(32);

    adapter.start(tx);

    controller.add_device("dev-1", 32);
    controller.add_device("dev-2", 64);
    controller.remove_device("dev-1");

    let mut events = Vec::new();
    for _ in 0..3 {
        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        events.push(event);
    }

    assert!(matches!(events[0], HardwareEvent::DeviceAdded(_)));
    assert!(matches!(events[1], HardwareEvent::DeviceAdded(_)));
    assert!(matches!(events[2], HardwareEvent::DeviceRemoved(_)));
}

#[tokio::test]
async fn test_list_devices_empty() {
    let (adapter, _controller) = SimulatedAdapter::new();
    let devices = adapter.list_devices().unwrap();
    assert!(devices.is_empty());
}

#[tokio::test]
async fn test_stop() {
    let (adapter, _controller) = SimulatedAdapter::new();
    let (tx, _rx) = mpsc::channel(32);

    adapter.start(tx);
    adapter.stop(); // Should not panic
}
