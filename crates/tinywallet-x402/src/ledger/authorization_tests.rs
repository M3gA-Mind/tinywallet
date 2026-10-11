//! Authorization retention and durable-write regressions for the ledger.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::*;

const SESSION: &str = "session-a";
static NEXT_TEST_RECORD: AtomicU64 = AtomicU64::new(1);

fn budget() -> SpendingBudget {
    SpendingBudget {
        per_request_max_atomic: 500_000,
        daily_max_atomic: 2_000_000,
        monthly_max_atomic: 10_000_000,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap()
}

fn record(
    amount: u64,
    status: PaymentStatus,
    timestamp: DateTime<Utc>,
    session: &str,
) -> PaymentRecord {
    PaymentRecord {
        id: format!(
            "id-{amount}-{}-{}",
            timestamp.timestamp(),
            NEXT_TEST_RECORD.fetch_add(1, Ordering::Relaxed)
        ),
        url: "https://api.example.com/data".into(),
        asset: "USDC".into(),
        amount_atomic: amount,
        amount_display: format!("{amount} atomic"),
        recipient: "Recipient".into(),
        network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
        tx_signature: Some("sig123".into()),
        status,
        timestamp,
        authorization_valid_until: None,
        session_id: session.into(),
        thread_id: None,
    }
}

fn ledger_in(dir: &tempfile::TempDir) -> PaymentLedger {
    PaymentLedger::new(dir.path(), SESSION, budget())
}

#[test]
fn expired_authorizations_stay_budgeted_after_rollover_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ledger_in(&dir);
    let authorized_at = now();
    let tomorrow = authorized_at + Duration::days(1);
    let next_month = authorized_at + Duration::days(20);
    let mut authorization = record(1_800_000, PaymentStatus::Pending, authorized_at, SESSION);
    authorization.id = "possibly-settled-before-expiry".to_string();
    authorization.authorization_valid_until = Some(authorized_at + Duration::hours(1));
    ledger.try_record_payment(authorization).unwrap();

    assert_eq!(
        ledger.usable_pending_count(tomorrow),
        0,
        "an expired proof no longer occupies a preparation slot"
    );
    assert_eq!(
        ledger.check_budget_at(tomorrow, 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        },
        "a lost response cannot let a possibly settled authorization release future budget"
    );
    assert_eq!(
        ledger.check_budget_at(next_month, 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        },
        "a possible prior settlement remains conservative across month rollover"
    );
    drop(ledger);

    let reopened = ledger_in(&dir);
    assert_eq!(
        reopened.check_budget_at(tomorrow, 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        },
        "restart preserves the unresolved authorization hold"
    );
    assert_eq!(
        reopened.check_budget_at(next_month, 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        }
    );
}

#[test]
fn legacy_expiry_failed_rows_remain_conservatively_budgeted() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ledger_in(&dir);
    let mut legacy = record(1_800_000, PaymentStatus::Failed, now(), SESSION);
    legacy.id = "legacy-expiry-failed".to_string();
    legacy.authorization_valid_until = Some(now() - Duration::seconds(1));
    ledger.try_record_payment(legacy).unwrap();

    assert_eq!(
        ledger.check_budget_at(now(), 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        },
        "older releases wrote Failed for timeout without settlement evidence"
    );
    drop(ledger);
    let reopened = ledger_in(&dir);
    assert_eq!(
        reopened.outstanding_atomic(),
        1_800_000,
        "the conservative hold survives process restart"
    );
}

#[test]
fn pending_totals_saturate_instead_of_wrapping() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ledger_in(&dir);
    ledger.record_payment(record(u64::MAX - 5, PaymentStatus::Pending, now(), SESSION));
    ledger.record_payment(record(10, PaymentStatus::Pending, now(), SESSION));
    assert_eq!(
        ledger.check_budget_at(now(), 1),
        BudgetCheck::ExceedsDailyBudget {
            current: u64::MAX,
            cap: 2_000_000
        }
    );
}

#[test]
fn adding_an_amount_past_the_maximum_cap_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = PaymentLedger::new(
        dir.path(),
        SESSION,
        SpendingBudget {
            per_request_max_atomic: u64::MAX,
            daily_max_atomic: u64::MAX,
            monthly_max_atomic: u64::MAX,
        },
    );
    ledger.record_payment(record(
        u64::MAX - 10,
        PaymentStatus::Pending,
        now(),
        SESSION,
    ));

    assert_eq!(
        ledger.check_budget_at(now(), 20),
        BudgetCheck::ExceedsDailyBudget {
            current: u64::MAX - 10,
            cap: u64::MAX
        },
        "an addition that overflows a maximal configured cap must not wrap into admission"
    );
}

#[test]
fn durable_append_separates_a_trailing_partial_jsonl_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ledger_in(&dir);
    let mut first = record(100_000, PaymentStatus::Pending, now(), SESSION);
    first.id = "first".to_string();
    ledger.try_record_payment(first).unwrap();

    let path = dir.path().join("x402/payments.jsonl");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"partial\"")
        .unwrap();

    let mut second = record(200_000, PaymentStatus::Pending, now(), SESSION);
    second.id = "durable-after-partial".to_string();
    ledger.try_record_payment(second).unwrap();

    let reopened = ledger_in(&dir);
    assert_eq!(
        reopened
            .payment("durable-after-partial")
            .unwrap()
            .amount_atomic,
        200_000,
        "a previous interrupted append cannot hide a later exported authorization"
    );
    assert_eq!(reopened.pending_count(), 2);
}

#[test]
fn the_latest_status_for_an_authorization_controls_budget_and_summary() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ledger_in(&dir);
    let mut authorization = record(1_800_000, PaymentStatus::Pending, now(), SESSION);
    authorization.id = "same-authorization".to_string();
    ledger.record_payment(authorization.clone());

    authorization.status = PaymentStatus::Settled;
    ledger.record_payment(authorization.clone());
    assert_eq!(
        ledger.check_budget_at(now(), 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        }
    );
    assert_eq!(ledger.summary_at(now()).daily_total_atomic, 1_800_000);

    authorization.status = PaymentStatus::Failed;
    authorization.timestamp += Duration::seconds(1);
    ledger.record_payment(authorization);
    assert_eq!(ledger.check_budget_at(now(), 300_000), BudgetCheck::Allowed);
    assert_eq!(ledger.summary_at(now()), SpendingSummary::default());
    assert_eq!(
        ledger.recent_payments(3).len(),
        3,
        "history stays append-only"
    );
}

#[test]
fn a_durable_reservation_commit_survives_reopening_without_double_counting() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    init_global(dir.path(), SESSION, budget());

    reserve(5)
        .unwrap()
        .unwrap()
        .commit_durable(record(5, PaymentStatus::Pending, now(), SESSION))
        .unwrap();
    assert_eq!(with_ledger(PaymentLedger::reserved_atomic).unwrap(), 0);
    assert_eq!(with_ledger(PaymentLedger::pending_count).unwrap(), 1);
    assert_eq!(with_ledger(PaymentLedger::outstanding_atomic).unwrap(), 5);

    reset_global();
    init_global(dir.path(), SESSION, budget());
    assert_eq!(with_ledger(PaymentLedger::pending_count).unwrap(), 1);
    assert_eq!(with_ledger(PaymentLedger::outstanding_atomic).unwrap(), 5);
    reset_global();
}
