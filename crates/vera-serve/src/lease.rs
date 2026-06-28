//! Lease acquisition — higher-level wrapper over ModelSlot.
//!
//! Provides the async acquire path including Loading-state wait logic.
//! Phase 2: actual model loading via vera-core will replace the stubs below.

use crate::model_slot::{LoadedModel, ModelLease, ModelSlot, SlotState};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::Duration;

/// Acquire a lease on the model slot, loading the model if needed.
///
/// Phase 2 stub: transitions Empty→Loading→Ready with a placeholder model.
/// Real implementation calls vera-core embedding/reranker load APIs.
pub async fn acquire(slot: Arc<ModelSlot>, model_id: &str) -> Result<ModelLease> {
    // Phase 2 stub: try to get existing Ready state first.
    {
        let mut guard = slot.inner.lock().map_err(|_| anyhow!("slot mutex poisoned"))?;
        match &*guard {
            SlotState::Ready(m) => {
                let m = Arc::clone(m);
                slot.in_flight
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                drop(guard);
                return Ok(ModelLease::new(Arc::clone(&slot), m));
            }
            SlotState::Loading => {
                // In Phase 2: park on a tokio::sync::Notify or Condvar.
                // For the scaffold, we just error — prevents infinite wait in tests.
                return Err(anyhow!("model is loading; retry after a short delay"));
            }
            SlotState::Empty => {
                *guard = SlotState::Loading;
            }
        }
    }

    // Load the model (Phase 2 stub — returns a placeholder).
    let loaded = tokio::task::spawn_blocking({
        let model_id = model_id.to_string();
        move || -> Result<Arc<LoadedModel>> {
            // Phase 2: replace with vera-core::EmbeddingModel::load(&model_id)
            Ok(Arc::new(LoadedModel {
                model_id,
                dim: 768, // placeholder — real dim from model metadata
                embedding_profile_id: "stub-profile".to_string(),
            }))
        }
    })
    .await
    .map_err(|e| anyhow!("model load task panicked: {e}"))??;

    // Store Ready and increment in_flight under the mutex.
    let mut guard = slot.inner.lock().map_err(|_| anyhow!("slot mutex poisoned"))?;
    *guard = SlotState::Ready(Arc::clone(&loaded));
    slot.in_flight
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    drop(guard);

    Ok(ModelLease::new(slot, loaded))
}

/// Spawn the background eviction task for a slot.
///
/// Wakes at `idle_timeout_secs` intervals and evicts the model if
/// `in_flight == 0` and the idle deadline has passed.
pub fn spawn_eviction_task(slot: Arc<ModelSlot>, idle_timeout_secs: i64) {
    if idle_timeout_secs == -1 {
        return; // keep-forever: no eviction
    }
    let timeout = if idle_timeout_secs == 0 {
        return; // cold-load mode: lease release handles unload in Phase 2
    } else {
        Duration::from_secs(idle_timeout_secs as u64)
    };

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(timeout).await;

            let Ok(mut guard) = slot.inner.lock() else { break };
            if !matches!(&*guard, SlotState::Ready(_)) {
                continue;
            }
            let in_flight = slot.in_flight.load(std::sync::atomic::Ordering::Acquire);
            if in_flight == 0 {
                // Safe to evict: in_flight checked inside mutex.
                *guard = SlotState::Empty;
                tracing::info!("vera-serve: model evicted after idle timeout");
            }
        }
    });
}
