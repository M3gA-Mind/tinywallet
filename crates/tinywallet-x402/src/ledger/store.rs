//! The ledger itself: an in-memory record list backed by a JSONL file.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Datelike, Utc};
use log::{debug, warn};

use super::types::{
    BudgetCheck, BudgetRefusal, PaymentRecord, PaymentStatus, ReservationId, SpendingBudget,
    SpendingSummary,
};

const LOG_PREFIX: &str = "[x402::store]";

/// Source of [`ReservationId`]s. Process-wide rather than per ledger, so a hold
/// taken on a ledger that was later replaced can never release a hold on its
/// successor.
static NEXT_RESERVATION: AtomicU64 = AtomicU64::new(1);

/// Append-only payment history with budget enforcement.
#[derive(Debug)]
pub struct PaymentLedger {
    records: Vec<PaymentRecord>,
    file_path: PathBuf,
    budget: SpendingBudget,
    session_id: String,
    /// Amounts held for payments that are signed but not yet recorded. Memory
    /// only: a hold that outlives its process protects nothing.
    reservations: HashMap<ReservationId, u64>,
}

impl PaymentLedger {
    /// Open the ledger under `workspace_dir`, loading any existing records.
    ///
    /// The file is `<workspace_dir>/x402/payments.jsonl`; a missing file is an
    /// empty ledger.
    #[must_use]
    pub fn new(workspace_dir: &Path, session_id: &str, budget: SpendingBudget) -> Self {
        let file_path = workspace_dir.join("x402").join("payments.jsonl");
        let records = load_from_disk(&file_path);
        debug!(
            "{LOG_PREFIX} loaded {} existing payment records from {}",
            records.len(),
            file_path.display()
        );
        Self {
            records,
            file_path,
            budget,
            session_id: session_id.to_string(),
            reservations: HashMap::new(),
        }
    }

    /// The session this ledger was opened for: the one
    /// [`SpendingSummary::session_total_atomic`] counts.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Check `amount` against the per-request, daily and monthly limits.
    ///
    /// Settled payments, unresolved signed authorizations, and outstanding
    /// [reservations](Self::reserve) count toward the daily and monthly totals.
    #[must_use]
    pub fn check_budget(&self, amount: u64) -> BudgetCheck {
        self.check_budget_at(Utc::now(), amount)
    }

    /// [`check_budget`](Self::check_budget) as of `now`, so the day and month
    /// windows can be tested without waiting for a boundary.
    pub(crate) fn check_budget_at(&self, now: DateTime<Utc>, amount: u64) -> BudgetCheck {
        if amount > self.budget.per_request_max_atomic {
            return BudgetCheck::ExceedsPerRequest {
                requested: amount,
                cap: self.budget.per_request_max_atomic,
            };
        }

        let today = now.date_naive();
        let this_month = (now.year(), now.month());
        let reserved = self.reserved_atomic();

        let daily = self
            .latest_records()
            .into_iter()
            .filter(|record| {
                is_unresolved(record)
                    || (record.status == PaymentStatus::Settled
                        && record.timestamp.date_naive() == today)
            })
            .fold(reserved, |total, record| {
                total.saturating_add(record.amount_atomic)
            });
        if daily
            .checked_add(amount)
            .is_none_or(|total| total > self.budget.daily_max_atomic)
        {
            return BudgetCheck::ExceedsDailyBudget {
                current: daily,
                cap: self.budget.daily_max_atomic,
            };
        }

        let monthly = self
            .latest_records()
            .into_iter()
            .filter(|record| {
                is_unresolved(record)
                    || (record.status == PaymentStatus::Settled
                        && (record.timestamp.year(), record.timestamp.month()) == this_month)
            })
            .fold(reserved, |total, record| {
                total.saturating_add(record.amount_atomic)
            });
        if monthly
            .checked_add(amount)
            .is_none_or(|total| total > self.budget.monthly_max_atomic)
        {
            return BudgetCheck::ExceedsMonthlyBudget {
                current: monthly,
                cap: self.budget.monthly_max_atomic,
            };
        }

        BudgetCheck::Allowed
    }

    /// Check `amount` against the limits and, if it fits, hold it, all under
    /// the caller's exclusive borrow.
    ///
    /// This is the atomic form of [`check_budget`](Self::check_budget) followed
    /// by signing: two payments that each fit alone cannot both be admitted when
    /// together they exceed a cap, because the second check sees the first hold.
    /// The hold ends with [`release`](Self::release) or
    /// [`commit_reservation`](Self::commit_reservation).
    ///
    /// # Errors
    ///
    /// The [`BudgetRefusal`] that stopped `amount`; nothing is held then.
    pub fn reserve(&mut self, amount: u64) -> Result<ReservationId, BudgetRefusal> {
        self.reserve_at(Utc::now(), amount)
    }

    /// [`reserve`](Self::reserve) as of `now`.
    pub(crate) fn reserve_at(
        &mut self,
        now: DateTime<Utc>,
        amount: u64,
    ) -> Result<ReservationId, BudgetRefusal> {
        if let Some(refusal) = self.check_budget_at(now, amount).refusal() {
            return Err(refusal);
        }
        let id = ReservationId(NEXT_RESERVATION.fetch_add(1, Ordering::Relaxed));
        self.reservations.insert(id, amount);
        debug!("{LOG_PREFIX} reserved {amount} atomic as {id:?}");
        Ok(id)
    }

    /// Drop a hold without recording a payment. Releasing a hold that is
    /// already gone does nothing.
    pub fn release(&mut self, id: ReservationId) {
        if self.reservations.remove(&id).is_some() {
            debug!("{LOG_PREFIX} released {id:?}");
        }
    }

    /// Record `record` and drop the hold `id` in one step, so the amount counts
    /// exactly once: as held before, as `record` after.
    pub fn commit_reservation(&mut self, id: ReservationId, record: PaymentRecord) {
        self.record_payment(record);
        self.release(id);
    }

    /// The total currently held by reservations, in atomic units.
    #[must_use]
    pub fn reserved_atomic(&self) -> u64 {
        self.reservations
            .values()
            .fold(0, |total, amount| total.saturating_add(*amount))
    }

    /// Attempt to append `record` to disk, then keep it in memory regardless.
    ///
    /// A write failure is logged and the record is still kept in memory: the
    /// payment already happened, and losing the record from this session's
    /// budget would be worse than losing it from the file.
    pub fn record_payment(&mut self, record: PaymentRecord) {
        if let Err(error) = self.append_to_disk(&record) {
            warn!("{LOG_PREFIX} record append failed: {error}");
        }
        self.records.push(record);
    }

    /// Append a state transition and update memory only after the durable write
    /// succeeds. Use this for transitions that must not release an authorization
    /// unless the new state is persisted.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, file creation, writing, or syncing
    /// fails. The in-memory history is left unchanged.
    pub fn try_record_payment(&mut self, record: PaymentRecord) -> Result<(), String> {
        self.append_to_disk(&record)?;
        self.records.push(record);
        Ok(())
    }

    /// Commit a reservation into a durable record without an interval where
    /// neither the hold nor the record counts against the budget.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, file creation, writing, or syncing
    /// fails. The record is not added to memory and the reservation stays held.
    pub fn try_commit_reservation(
        &mut self,
        id: ReservationId,
        record: PaymentRecord,
    ) -> Result<(), String> {
        self.append_to_disk(&record)?;
        self.records.push(record);
        self.release(id);
        Ok(())
    }

    /// Totals over settled payments for this session, today and this month.
    #[must_use]
    pub fn summary(&self) -> SpendingSummary {
        self.summary_at(Utc::now())
    }

    /// [`summary`](Self::summary) as of `now`.
    pub(crate) fn summary_at(&self, now: DateTime<Utc>) -> SpendingSummary {
        let today = now.date_naive();
        let this_month = (now.year(), now.month());

        let mut summary = SpendingSummary::default();
        for record in self
            .latest_records()
            .into_iter()
            .filter(|r| r.status == PaymentStatus::Settled)
        {
            if record.session_id == self.session_id {
                summary.session_total_atomic = summary
                    .session_total_atomic
                    .saturating_add(record.amount_atomic);
                summary.session_count = summary.session_count.saturating_add(1);
            }
            if record.timestamp.date_naive() == today {
                summary.daily_total_atomic = summary
                    .daily_total_atomic
                    .saturating_add(record.amount_atomic);
                summary.daily_count = summary.daily_count.saturating_add(1);
            }
            if (record.timestamp.year(), record.timestamp.month()) == this_month {
                summary.monthly_total_atomic = summary
                    .monthly_total_atomic
                    .saturating_add(record.amount_atomic);
                summary.monthly_count = summary.monthly_count.saturating_add(1);
            }
        }
        summary
    }

    /// The most recent `limit` records, newest first.
    #[must_use]
    pub fn recent_payments(&self, limit: usize) -> Vec<PaymentRecord> {
        self.records.iter().rev().take(limit).cloned().collect()
    }

    /// Return the latest recorded state for a payment id.
    #[must_use]
    pub fn payment(&self, id: &str) -> Option<&PaymentRecord> {
        self.records.iter().rev().find(|record| record.id == id)
    }

    /// Number of authorizations whose latest state is still pending.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.latest_records()
            .into_iter()
            .filter(|record| record.status == PaymentStatus::Pending)
            .count()
    }

    /// Number of pending authorizations that can still be used to initiate a
    /// payment. Expiry can release an admission slot, but it never changes the
    /// durable budget hold: an expired authorization may already have settled
    /// without a receipt reaching this process.
    #[must_use]
    pub fn usable_pending_count(&self, now: DateTime<Utc>) -> usize {
        self.latest_records()
            .into_iter()
            .filter(|record| {
                record.status == PaymentStatus::Pending
                    && record
                        .authorization_valid_until
                        .is_none_or(|valid_until| valid_until > now)
            })
            .count()
    }

    /// Latest unresolved signed authorizations.
    #[must_use]
    pub fn pending_payments(&self) -> Vec<PaymentRecord> {
        self.latest_records()
            .into_iter()
            .filter(|record| record.status == PaymentStatus::Pending)
            .cloned()
            .collect()
    }

    /// Budget still tied up in reservations and durable pending authorizations.
    #[must_use]
    pub fn outstanding_atomic(&self) -> u64 {
        let pending = self
            .latest_records()
            .into_iter()
            .filter(|record| is_unresolved(record))
            .fold(0_u64, |total, record| {
                total.saturating_add(record.amount_atomic)
            });
        pending.saturating_add(self.reserved_atomic())
    }

    fn latest_records(&self) -> Vec<&PaymentRecord> {
        let mut latest = HashMap::with_capacity(self.records.len());
        for record in &self.records {
            latest.insert(record.id.as_str(), record);
        }
        latest.into_values().collect()
    }

    /// The current limits.
    #[must_use]
    pub fn budget(&self) -> &SpendingBudget {
        &self.budget
    }

    /// Replace the limits. Not persisted; a host re-applies its own defaults at
    /// boot.
    pub fn update_budget(&mut self, budget: SpendingBudget) {
        debug!(
            "{LOG_PREFIX} budget updated per_request={} daily={} monthly={}",
            budget.per_request_max_atomic, budget.daily_max_atomic, budget.monthly_max_atomic
        );
        self.budget = budget;
    }

    fn append_to_disk(&self, record: &PaymentRecord) -> Result<(), String> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create ledger directory: {error}"))?;
        }
        let mut file = match OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.file_path)
        {
            Ok(f) => f,
            Err(error) => return Err(format!("open ledger file: {error}")),
        };
        let json = serde_json::to_string(record)
            .map_err(|error| format!("serialize ledger record: {error}"))?;
        let length = file
            .metadata()
            .map_err(|error| format!("inspect ledger file: {error}"))?
            .len();
        if length > 0 {
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("seek ledger file: {error}"))?;
            let mut last = [0_u8; 1];
            file.read_exact(&mut last)
                .map_err(|error| format!("read ledger file boundary: {error}"))?;
            if last[0] != b'\n' {
                file.write_all(b"\n")
                    .map_err(|error| format!("finish ledger line: {error}"))?;
            }
        }
        writeln!(file, "{json}").map_err(|error| format!("write ledger record: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync ledger record: {error}"))?;
        #[cfg(unix)]
        if let Some(parent) = self.file_path.parent() {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("sync ledger directory: {error}"))?;
        }
        Ok(())
    }
}

fn is_unresolved(record: &PaymentRecord) -> bool {
    record.status == PaymentStatus::Pending
        // Older module versions marked an authorization Failed solely because
        // its valid-before deadline elapsed. Keep those historical rows
        // budgeted until a trusted receipt resolves them.
        || (record.status == PaymentStatus::Failed
            && record.authorization_valid_until.is_some())
}

fn load_from_disk(path: &Path) -> Vec<PaymentRecord> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(l) => l,
            // Invalid UTF-8 consumes the line, so the next read moves on.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                warn!("{LOG_PREFIX} skipping unreadable line: {e}");
                continue;
            }
            // Any other error repeats forever on the same position (a directory
            // where the file should be, a failing disk), so stop reading.
            Err(e) => {
                warn!("{LOG_PREFIX} read error, stopping load: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PaymentRecord>(&line) {
            Ok(r) => records.push(r),
            Err(e) => warn!("{LOG_PREFIX} corrupt record: {e}"),
        }
    }
    records
}
