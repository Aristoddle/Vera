//! ModelSlot — the core concurrency primitive for warm model lifecycle.
//!
//! Design: Mutex<SlotState> guards all state transitions.
//! `in_flight` is incremented INSIDE the mutex before the slot's `Ready` state
//! is released to callers, ensuring the eviction task never observes `in_flight==0`
//! on a slot that still has a live caller.
//!
//! Release ordering: `last_used` is updated BEFORE `in_flight` is decremented,
//! both under the same mutex lock. This prevents eviction from observing a stale
//! `last_used` timestamp after a long request completes.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A loaded model and its metadata.
pub struct LoadedModel {
    // The actual model handle lives here in Phase 2 implementation.
    // Placeholder until vera-core embedding/reranker types are wired.
    pub model_id: String,
    pub dim: usize,
    pub embedding_profile_id: String,
}

/// Slot state machine.
pub enum SlotState {
    /// No model loaded.
    Empty,
    /// Model is being loaded; other threads should wait.
    Loading,
    /// Model is loaded and ready.
    Ready(Arc<LoadedModel>),
}

/// Thread-safe warm model slot.
///
/// # Invariants
/// - `in_flight` is only incremented while `inner` mutex is held.
/// - `last_used` is updated before `in_flight` is decremented (both under mutex).
/// - Eviction checks both conditions inside the mutex.
pub struct ModelSlot {
    /// Guards state transitions AND in_flight increment/decrement.
    pub inner: Mutex<SlotState>,
    /// Number of active leases. Only modified under `inner` lock.
    pub in_flight: AtomicU32,
    /// Epoch milliseconds of last lease release. Updated before in_flight-- under lock.
    pub last_used: AtomicU64,
}

impl ModelSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(SlotState::Empty),
            in_flight: AtomicU32::new(0),
            last_used: AtomicU64::new(epoch_ms()),
        })
    }
}

/// RAII lease guard. Decrements `in_flight` and updates `last_used` on drop.
pub struct ModelLease {
    slot: Arc<ModelSlot>,
    model: Arc<LoadedModel>,
}

impl ModelLease {
    /// Construct a lease. Only called from `lease::acquire` while the slot mutex is held.
    pub(crate) fn new(slot: Arc<ModelSlot>, model: Arc<LoadedModel>) -> Self {
        Self { slot, model }
    }

    pub fn model(&self) -> &Arc<LoadedModel> {
        &self.model
    }
}

impl Drop for ModelLease {
    fn drop(&mut self) {
        // Update last_used BEFORE decrementing in_flight.
        // Both happen under the inner mutex to close the release/evict race.
        let _guard = self.slot.inner.lock().unwrap();
        self.slot.last_used.store(epoch_ms(), Ordering::Relaxed);
        self.slot.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_starts_empty() {
        let slot = ModelSlot::new();
        let guard = slot.inner.lock().unwrap();
        assert!(matches!(*guard, SlotState::Empty));
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
    }
}
