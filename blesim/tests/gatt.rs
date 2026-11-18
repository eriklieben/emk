//! GATT over the simulated air: the right half's key bitmap, notified to the
//! left half over an encrypted link (as the firmware's split service).

mod common;

use blesim::{Air, SimController};
use common::*;
use embassy_futures::join::join;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

#[gatt_server]
struct SplitServer {
    split: SplitService,
}

#[gatt_service(uuid = "8f0e0001-6b7d-4a3c-9e2f-5c1d2b3a4e01")]
struct SplitService {
    /// Held keys of the right half: bit n = Corne position n.
    #[characteristic(
        uuid = "8f0e0002-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        read,
        notify,
        permissions(encrypted)
    )]
    keys: [u8; 8],
}

const HELD: u64 = 0x0000_0200_0400_0081;

#[test]
fn notification_from_the_peripheral_reaches_the_subscribed_central() {
    // Arrange
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let server = SplitServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "right",
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let received = run(
        join(runner(&left), runner(&right)),
        select(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(true).unwrap();
                conn.request_security().unwrap();
                secured(&conn).await;
                subscribe_and_receive(&left, &conn).await
            },
            async {
                let conn = advertise(&mut peripheral).await;
                conn.set_bondable(true).unwrap();
                let conn = conn.with_attribute_server(&server).unwrap();
                serve_and_notify(&server, &conn).await;
            },
        ),
    );

    // Assert
    assert!(
        matches!(received, Either::First(HELD)),
        "received {received:?}"
    );
}

/// The left half's side: find the split service, subscribe to the keys and
/// return the first notified bitmap.
async fn subscribe_and_receive(
    stack: &HostStack<'_>,
    conn: &Connection<'_, DefaultPacketPool>,
) -> u64 {
    let client = GattClient::<SimController, DefaultPacketPool, 4>::new(stack, conn)
        .await
        .expect("gatt client");
    let receive = async {
        let services = client
            .services_by_uuid(&Uuid::new_long(SPLIT_SERVICE))
            .await
            .expect("discover service");
        let service = services.first().expect("split service").clone();
        let mut uuid = SPLIT_SERVICE;
        uuid[12] = 0x02;
        let keys = client
            .characteristic_by_uuid::<[u8; 8]>(&service, &Uuid::new_long(uuid))
            .await
            .expect("discover characteristic");
        let mut listener = client.subscribe(&keys, false).await.expect("subscribe");
        let data = listener.next().await;
        let bytes: [u8; 8] = data.as_ref().try_into().expect("8 bytes");
        u64::from_le_bytes(bytes)
    };
    match select(client.task(), receive).await {
        Either::First(result) => panic!("gatt client stopped: {result:?}"),
        Either::Second(bits) => bits,
    }
}

/// The right half's side: answer GATT requests and pairing, and notify the
/// held keys every few milliseconds (unsent until the left half subscribed).
async fn serve_and_notify(
    server: &SplitServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) {
    let events = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => panic!("disconnected: {reason:?}"),
                GattConnectionEvent::PairingFailed(e) => panic!("pairing failed: {e:?}"),
                GattConnectionEvent::PassKeyConfirm(_) => conn.pass_key_confirm().unwrap(),
                GattConnectionEvent::Gatt { event } => {
                    if let Ok(reply) = event.accept() {
                        reply.send().await;
                    }
                }
                _ => {}
            }
        }
    };
    let notify = async {
        loop {
            let _ = server
                .split
                .keys
                .notify(conn, &HELD.to_le_bytes(), true)
                .await;
            Timer::after(Duration::from_millis(10)).await;
        }
    };
    select(events, notify).await;
}
