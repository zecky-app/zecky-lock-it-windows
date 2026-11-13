//! The onion address is made once and kept. Runs without a network.

use lockit_tor::OnionAccess;

#[test]
fn the_address_is_kept_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let (state, cache) = (dir.path().join("state"), dir.path().join("cache"));

    let first = OnionAccess::start(&state, &cache, 8137).unwrap();
    let address = first.address().to_owned();
    first.stop();

    assert!(address.ends_with(".onion"), "{address}");
    assert_eq!(address.len(), 56 + ".onion".len(), "a v3 onion address: {address}");

    let second = OnionAccess::start(&state, &cache, 8137).unwrap();
    assert_eq!(second.address(), address, "the same install must keep its address");
    second.stop();

    // A different install (another state folder) gets a different address.
    let other_dir = tempfile::tempdir().unwrap();
    let other = OnionAccess::start(&other_dir.path().join("state"), &other_dir.path().join("cache"), 8137).unwrap();
    assert_ne!(other.address(), address);
    other.stop();
}
