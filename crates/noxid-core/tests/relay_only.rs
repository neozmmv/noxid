//! "Use only this relay" against real relay servers on loopback: one stands
//! in for n0's relays, the other is the custom relay. Checks that the map
//! follows the setting, that the endpoint re-homes at runtime, and that two
//! endpoints with only the custom relay reach each other through a code.

use std::collections::BTreeSet;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl, Watcher};
use noxid_core::address_book::{AddressBook, Source};
use noxid_core::relay::CustomRelay;
use noxid_core::room::{Code, room_code};

const ALPN: &[u8] = b"noxid-test/relay-only";
/// iroh re-homes after one net report; its own test of a map change allows
/// ten seconds.
const REHOME: Duration = Duration::from_secs(15);

async fn endpoint(map: RelayMap, book: Option<&AddressBook>) -> Endpoint {
    let mut builder = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .alpns(vec![ALPN.to_vec()]);
    if let Some(book) = book {
        builder = builder.address_lookup(book.clone());
    }
    builder.bind().await.unwrap()
}

/// Waits for the endpoint's home relay to be `want`.
async fn homed_on(endpoint: &Endpoint, want: &RelayUrl) {
    let mut addrs = endpoint.watch_addr();
    let started = std::time::Instant::now();
    tokio::time::timeout(REHOME, async {
        loop {
            if addrs.get().relay_urls().next() == Some(want) {
                return;
            }
            addrs.updated().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("not homed on {want} in time: {:?}", endpoint.addr()));
    eprintln!("homed on {want} after {:?}", started.elapsed());
}

#[tokio::test]
async fn the_map_follows_the_setting_and_the_endpoint_rehomes() {
    let (n0_map, n0_url, _n0) = iroh::test_utils::run_relay_server().await.unwrap();
    let (_, custom_url, _custom) = iroh::test_utils::run_relay_server().await.unwrap();

    // As a node starts with the setting off: n0's map, the custom relay added.
    let relays = CustomRelay::new(n0_map.clone(), None, false);
    let ep = endpoint(relays.initial_map(), None).await;
    homed_on(&ep, &n0_url).await;
    relays.set(&ep, Some(custom_url.clone())).await;
    assert_eq!(relays.applied().await, BTreeSet::from([n0_url.clone(), custom_url.clone()]));

    relays.set_only(&ep, true).await.unwrap();
    assert_eq!(relays.applied().await, BTreeSet::from([custom_url.clone()]));
    homed_on(&ep, &custom_url).await;

    relays.set_only(&ep, false).await.unwrap();
    assert_eq!(relays.applied().await, BTreeSet::from([n0_url.clone(), custom_url.clone()]));

    // Removing the relay while on turns "only" off and brings n0's back.
    relays.set_only(&ep, true).await.unwrap();
    relays.set(&ep, None).await;
    assert!(!relays.only());
    assert_eq!(relays.applied().await, BTreeSet::from([n0_url.clone()]));
    homed_on(&ep, &n0_url).await;

    // And "only" without a relay is refused.
    assert!(relays.set_only(&ep, true).await.is_err());
    ep.close().await;
}

#[tokio::test]
async fn changing_the_relay_keeps_only_on() {
    let (n0_map, _n0_url, _n0) = iroh::test_utils::run_relay_server().await.unwrap();
    let (_, first, _a) = iroh::test_utils::run_relay_server().await.unwrap();
    let (_, second, _b) = iroh::test_utils::run_relay_server().await.unwrap();

    let relays = CustomRelay::new(n0_map, Some(first.clone()), true);
    let ep = endpoint(relays.initial_map(), None).await;
    homed_on(&ep, &first).await;

    relays.set(&ep, Some(second.clone())).await;
    assert!(relays.only());
    assert_eq!(relays.applied().await, BTreeSet::from([second.clone()]));
    homed_on(&ep, &second).await;
    ep.close().await;
}

#[tokio::test]
async fn two_endpoints_on_only_the_custom_relay_meet_through_a_code() {
    let (n0_map, n0_url, _n0) = iroh::test_utils::run_relay_server().await.unwrap();
    let (_, custom_url, _custom) = iroh::test_utils::run_relay_server().await.unwrap();

    // Both start with the setting on, as a saved profile would have it.
    let host_relays = CustomRelay::new(n0_map.clone(), Some(custom_url.clone()), true);
    let host = endpoint(host_relays.initial_map(), None).await;
    let joiner_relays = CustomRelay::new(n0_map, Some(custom_url.clone()), true);
    let book = AddressBook::new(iroh::SecretKey::generate().public());
    let joiner = endpoint(joiner_relays.initial_map(), Some(&book)).await;

    // The first home relay is already the custom one: n0's was never in the map.
    homed_on(&host, &custom_url).await;
    assert!(!host.addr().relay_urls().any(|url| *url == n0_url));

    // The host's code names the custom relay, and the joiner dials by id
    // with nothing but what the code said.
    let home = host.addr().relay_urls().next().cloned();
    let code = room_code(&host.id(), home.as_ref());
    let decoded = Code::decode(&code).unwrap();
    assert_eq!(decoded.relay(), Some(&custom_url));
    book.insert(decoded.coordinator(), decoded.relay().unwrap().clone(), Source::Code);

    let accept = tokio::spawn({
        let host = host.clone();
        async move {
            let conn = host.accept().await.unwrap().await.unwrap();
            let (mut send, mut recv) = conn.accept_bi().await.unwrap();
            let data = recv.read_to_end(64).await.unwrap();
            send.write_all(&data).await.unwrap();
            send.finish().unwrap();
            conn.closed().await;
        }
    });
    let conn = tokio::time::timeout(REHOME, joiner.connect(decoded.coordinator(), ALPN))
        .await
        .expect("connect timed out")
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"hello").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"hello");
    conn.close(0u32.into(), b"done");
    let _ = tokio::time::timeout(Duration::from_secs(5), accept).await;

    host.close().await;
    joiner.close().await;
}
