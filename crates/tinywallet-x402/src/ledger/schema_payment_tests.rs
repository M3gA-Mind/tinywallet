#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn rfc3339_milliseconds_handle_offsets_and_truncate_submillisecond_precision() {
    assert_eq!(
        parse_rfc3339_ms("2026-03-15T12:00:00.250Z"),
        Some(1_773_576_000_250)
    );
    assert_eq!(
        parse_rfc3339_ms("2026-03-15t15:00:00.250000+03:00"),
        Some(1_773_576_000_250)
    );
    assert_eq!(
        parse_rfc3339_ms("2026-03-15 12:00:00.9z"),
        Some(1_773_576_000_900)
    );
    assert_eq!(
        parse_rfc3339_ms("2026-03-15T12:00:60Z"),
        Some(1_773_576_060_000)
    );
    assert!(parse_rfc3339_ms("2024-02-29T00:00:00Z").is_some());
}

#[test]
fn rfc3339_parser_rejects_malformed_fields_and_offsets() {
    for value in [
        "2026-13-15T12:00:00Z",
        "2026-03-00T12:00:00Z",
        "2026-02-29T12:00:00Z",
        "2024-02-31T12:00:00Z",
        "2026-03-15T24:00:00Z",
        "2026-03-15T12:60:00Z",
        "2026-03-15T12:00:61Z",
        "2026-03-15T12:00:00.",
        "2026-03-15T12:00:00+01:0x",
        "2026-03-15T12:00:00+24:00",
        "2026-03-15T12:00:00+01:60",
        "2026-03-15T12:00:00+01:00tail",
        "2026/03/15T12:00:00Z",
    ] {
        assert_eq!(parse_rfc3339_ms(value), None, "{value}");
    }
}

#[test]
fn legacy_payment_deserialization_defaults_absent_optional_fields_and_rejects_bad_instants() {
    let base = serde_json::json!({
        "id":"id", "url":"https://example.test", "asset":"USD", "amountAtomic":1,
        "amountDisplay":"1", "recipient":"r", "network":"n", "status":"pending",
        "timestamp":"2026-03-15T12:00:00Z", "sessionId":"s"
    });
    let legacy: LegacyPayment = serde_json::from_value(base.clone()).unwrap();
    let stored = StoredPayment::from(legacy);
    assert_eq!(stored.timestamp_ms, 1_773_576_000_000);
    assert_eq!(stored.tx_signature, None);
    assert_eq!(stored.authorization_valid_until_ms, None);
    assert_eq!(stored.solana_last_valid_block_height, None);
    assert_eq!(stored.authorization_expired_at_ms, None);
    assert_eq!(stored.thread_id, None);
    let mut with_optionals = serde_json::json!({
        "id":"id", "url":"https://example.test", "asset":"USD", "amountAtomic":1,
        "amountDisplay":"1", "recipient":"r", "network":"n", "status":"pending",
        "timestamp":"2026-03-15T12:00:00Z", "sessionId":"s",
        "authorizationValidUntil":"2026-03-15T12:00:01.9999Z",
        "authorizationExpiredAt":"2026-03-15T12:00:02Z",
        "solanaLastValidBlockHeight":42, "authorizationExpiryObservedHeight":43,
        "threadId":"thread", "committed_hold":"hold", "txSignature":"sig"
    });
    let parsed: LegacyPayment = serde_json::from_value(with_optionals.clone()).unwrap();
    let parsed = StoredPayment::from(parsed);
    assert_eq!(parsed.authorization_valid_until_ms, Some(1_773_576_001_999));
    assert_eq!(parsed.authorization_expired_at_ms, Some(1_773_576_002_000));
    assert_eq!(parsed.solana_last_valid_block_height, Some(42));
    assert_eq!(parsed.authorization_expiry_observed_height, Some(43));
    assert_eq!(parsed.thread_id.as_deref(), Some("thread"));
    assert_eq!(parsed.committed_hold.as_deref(), Some("hold"));
    assert_eq!(parsed.tx_signature.as_deref(), Some("sig"));
    with_optionals["authorizationValidUntil"] = serde_json::json!("tomorrow");
    assert!(serde_json::from_value::<LegacyPayment>(with_optionals).is_err());
    assert!(
        serde_json::from_value::<LegacyPayment>({
            let mut bad = base;
            bad["timestamp"] = serde_json::json!("yesterday");
            bad
        })
        .is_err()
    );
}
