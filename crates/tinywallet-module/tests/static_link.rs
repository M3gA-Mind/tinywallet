//! The `static-link` build exports `linked_module()` and the host admits it
//! in-process, attesting it as host code (the executable's digest).

use sha2::{Digest, Sha256};
use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinywallet_module::{BUS_NAME, linked_module};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_module_is_admitted_and_attested_as_the_host_executable() {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);

    let module = linked_module().expect("linked manifest is valid");
    let info = host
        .attach_linked_with_config(module, serde_json::json!({}))
        .expect("linked tinywallet is admitted");
    assert_eq!(info.manifest.bus_name.as_ref() as &str, BUS_NAME);
    assert_eq!(info.state, ModuleState::Resolved);

    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let attestation = client
        .attestation(info.manifest.bus_name.clone())
        .await
        .unwrap()
        .expect("linked module is attested, so confidential members are deliverable");
    let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    let hex: String = Sha256::digest(exe)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(attestation.sha256, hex);
    broker_task.abort();
}
