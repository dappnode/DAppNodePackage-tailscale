use super::*;
use serde_json::{Value, json};

fn snapshot(state: &str, url: Value) -> Snapshot {
    Snapshot::from_json(
        &serde_json::to_vec(&json!({
            "BackendState": state, "AuthURL": url, "Self": {"Online": true}
        }))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn reports_login_url_only_while_needing_login() {
    let url = "https://login.tailscale.com/a/synthetic?x=1&y=a%2Bb";
    let needs_login = snapshot("NeedsLogin", json!(url));
    assert_eq!(needs_login.auth_url.as_ref().map(AsRef::as_ref), Some(url));
    assert!(!needs_login.online);
    for state in [
        "NoState",
        "NeedsMachineAuth",
        "Stopped",
        "Starting",
        "Running",
        "FutureState",
    ] {
        assert!(snapshot(state, json!(url)).auth_url.is_none());
    }
    assert!(snapshot("Running", json!(url)).online);
}

#[test]
fn missing_or_null_self_is_valid_while_unauthenticated() {
    for input in [
        r#"{"BackendState":"NeedsLogin"}"#,
        r#"{"BackendState":"NeedsLogin","Self":null,"AuthURL":null}"#,
    ] {
        let snapshot = Snapshot::from_json(input.as_bytes()).unwrap();
        assert!(snapshot.state == State::NeedsLogin);
        assert!(snapshot.auth_url.is_none());
        assert!(!snapshot.online);
    }
}

#[test]
fn unknown_states_fail_closed() {
    assert!(snapshot("FutureState", Value::Null) == Snapshot::unavailable());
}

#[test]
fn rejects_malformed_status_instead_of_reusing_a_link() {
    for input in [
        "",
        "garbage",
        "{}",
        "null",
        "[]",
        "{} {}",
        r#"{"BackendState":123}"#,
        r#"{"BackendState":"Running","Self":{"Online":"true"}}"#,
        r#"{"BackendState":"NeedsLogin","AuthURL":123}"#,
    ] {
        assert!(matches!(
            Snapshot::from_json(input.as_bytes()),
            Err(Error::InvalidStatus)
        ));
    }
}

#[test]
fn rejects_unsafe_and_oversized_urls_without_losing_login_state() {
    for url in [
        "http://login.tailscale.com/a/test",
        "https://login.tailscale.com.attacker.test/a/test",
        "https://login.tailscale.com@attacker.test/a/test",
        "https://attacker.test/a/test",
        "https://login.tailscale.com/admin/machines",
        "https://login.tailscale.com/a/",
        "https://login.tailscale.com/a/test#fragment",
        "https://login.tailscale.com/a/test\n",
        "https://login.tailscale.com/a/test\\evil",
        "https://login.tailscale.com/a/\"test",
        "https://login.tailscale.com/a/é",
        &format!("https://login.tailscale.com/a/{}", "x".repeat(512)),
    ] {
        let snapshot = snapshot("NeedsLogin", json!(url));
        assert!(snapshot.state == State::NeedsLogin);
        assert!(snapshot.auth_url.is_none());
    }
}

#[test]
fn largest_payload_fits_existing_data_send_limit() {
    let prefix = "https://login.tailscale.com/a/";
    let url = format!("{prefix}{}", "x".repeat(512 - prefix.len()));
    let snapshot = snapshot("NeedsLogin", json!(url));
    assert!(snapshot.auth_url.is_some());
    let body = snapshot.payload(u64::MAX).unwrap();
    assert!(body.len() <= MAX_PAYLOAD_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["authUrl"], url);
    assert_eq!(payload["updatedAt"], u64::MAX);
    assert_eq!(payload["version"], 1);
    assert_eq!(payload.as_object().unwrap().len(), 5);
}

#[test]
fn manager_endpoint_requires_an_internal_http_origin() {
    assert_eq!(
        DEFAULT_MANAGER
            .parse::<ManagerEndpoint>()
            .unwrap()
            .as_url()
            .as_str(),
        "http://dappmanager.dappnode/data-send?key=tailscale.status"
    );
    for invalid in [
        "invalid",
        "https://dappmanager.dappnode",
        "http://user:secret@host",
        "http://host/path",
        "http://host?query",
        "http://host#fragment",
    ] {
        assert!(invalid.parse::<ManagerEndpoint>().is_err());
    }
}
