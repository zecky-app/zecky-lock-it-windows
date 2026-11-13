//! The config files Go Max writes.

use lockit_node::{GoMaxSettings, Listening, NodeLayout};

fn parse(text: &str) -> toml::Table {
    text.parse().expect("the written config must be valid TOML")
}

#[test]
fn zakura_is_pruned_private_and_cookie_protected() {
    let layout = NodeLayout::new(r"C:\Users\Owner\AppData\Local\Zecky\Lock it\go-max");
    let config = parse(&GoMaxSettings::default().zakura_toml(&layout).unwrap());

    assert_eq!(config["state"]["storage_mode"]["pruned"]["tx_retention"].as_integer(), Some(10_000));
    assert_eq!(config["consensus"]["checkpoint_sync"].as_bool(), Some(false));
    assert_eq!(config["network"]["p2p_stack"].as_str(), Some("legacy"));
    let state = config["state"]["cache_dir"].as_str().unwrap();
    assert!(state.starts_with(r"C:\Users\Owner\AppData\Local\Zecky\Lock it\go-max"), "{state}");
    assert_eq!(config["network"]["network"].as_str(), Some("Mainnet"));
    // Private: listens on this computer only, and never on IPv6 "[::]".
    assert_eq!(config["network"]["listen_addr"].as_str(), Some("127.0.0.1:8233"));
    assert!(config["network"].get("zakura").is_none(), "no second P2P port");
    assert_eq!(config["rpc"]["listen_addr"].as_str(), Some("127.0.0.1:8232"));
    assert_eq!(config["rpc"]["enable_cookie_auth"].as_bool(), Some(true));
    assert!(config["network"].get("peerset_initial_target_size").is_none());
}

#[test]
fn helping_the_network_listens_on_all_ipv4_addresses() {
    let layout = NodeLayout::new("/tmp/go-max");
    let settings = GoMaxSettings { listening: Listening::HelpTheNetwork, peer_target: Some(150), ..Default::default() };
    let config = parse(&settings.zakura_toml(&layout).unwrap());
    assert_eq!(config["network"]["listen_addr"].as_str(), Some("0.0.0.0:8233"));
    assert_eq!(config["network"]["peerset_initial_target_size"].as_integer(), Some(150));
    // RPC stays on this computer whatever the owner chooses.
    assert_eq!(config["rpc"]["listen_addr"].as_str(), Some("127.0.0.1:8232"));
}

#[test]
fn zaino_reads_zakura_with_the_cookie_and_serves_on_loopback() {
    let layout = NodeLayout::new("/tmp/go-max");
    let config = parse(&GoMaxSettings::default().zaino_toml(&layout).unwrap());
    assert_eq!(config["backend"].as_str(), Some("rpc"));
    assert_eq!(config["ephemeral_finalised_state"].as_bool(), Some(true));
    assert_eq!(config["network"].as_str(), Some("Mainnet"));
    assert_eq!(config["grpc_settings"]["listen_address"].as_str(), Some("127.0.0.1:8137"));
    assert_eq!(
        config["validator_settings"]["validator_jsonrpc_listen_address"].as_str(),
        Some("127.0.0.1:8232")
    );
    let cookie = config["validator_settings"]["validator_cookie_path"].as_str().unwrap();
    assert_eq!(std::path::Path::new(cookie), layout.cookie_file());
    // No password is ever written.
    assert!(config["validator_settings"].get("validator_password").is_none());
}

#[test]
fn windows_paths_survive_the_round_trip() {
    // Backslashes and spaces must come back exactly as written.
    let root = r"C:\Users\Owner Name\AppData\Local\Zecky\Lock it\go-max";
    let layout = NodeLayout::new(root);
    let config = parse(&GoMaxSettings::default().zaino_toml(&layout).unwrap());
    let path = config["storage"]["database"]["path"].as_str().unwrap();
    assert!(path.starts_with(root), "{path}");
}

#[test]
fn folders_are_created_and_configs_written() {
    let dir = tempfile::tempdir().unwrap();
    let layout = NodeLayout::new(dir.path().join("go-max"));
    layout.create().unwrap();
    GoMaxSettings::default().write_configs(&layout).unwrap();
    assert!(layout.zakura_config().is_file());
    assert!(layout.zaino_config().is_file());
    assert!(layout.zakura_state().is_dir());
    assert!(layout.zaino_database().is_dir());
    assert!(layout.log_dir().is_dir());
}

#[test]
fn a_busy_port_is_named_before_starting() {
    let dir = tempfile::tempdir().unwrap();
    let layout = NodeLayout::new(dir.path().join("go-max"));
    layout.create().unwrap();
    // Hold a port, then ask Go Max to use it for the wallet service.
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let settings = GoMaxSettings { wallet_port: port, rpc_port: 0, p2p_port: 0, ..Default::default() };
    match lockit_node::checks::before_start(&layout, &settings) {
        Err(lockit_node::Error::PortInUse { port: busy, .. }) => assert_eq!(busy, port),
        Err(lockit_node::Error::NotEnoughSpace { .. }) => {} // a small test disk; the port check comes after
        other => panic!("expected the busy port to be named, got {other:?}"),
    }
}

#[test]
fn free_space_is_read() {
    let dir = tempfile::tempdir().unwrap();
    // A folder that does not exist yet is measured on its nearest parent.
    let free = lockit_node::checks::free_bytes(&dir.path().join("not-yet/created")).unwrap();
    assert!(free > 0);
}
