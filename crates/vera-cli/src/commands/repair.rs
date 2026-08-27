//! `vera repair` — repair the configured backend by re-fetching missing assets.

use vera_core::config::InferenceBackend;
use vera_core::local_models::LocalEmbeddingModelConfig;

use crate::commands::setup;
use crate::state;

/// The embedding model `vera repair` hands to the asset preparation path.
///
/// Deliberately the raw stored value, not `state::repaired_local_embedding_model`:
/// an asset repair does not need the in-memory pooling repair, and it must not
/// accidentally turn that runtime-only correction into persisted configuration.
///
/// Runtime readers still receive the corrected model through
/// `state::apply_saved_env_force`; asset prefetching never reads `pooling`.
/// `pub(crate)` so the regression test can live beside the other stored-pooling
/// tests in `state`, which own the `VERA_HOME` lock this needs.
pub(crate) fn embedding_model_for_repair(
    effective_backend: InferenceBackend,
) -> anyhow::Result<Option<LocalEmbeddingModelConfig>> {
    if !effective_backend.is_onnx() {
        return Ok(None);
    }
    Ok(Some(
        state::load_saved_config()?
            .local_embedding_model
            .unwrap_or_default(),
    ))
}

pub fn run(backend: Option<InferenceBackend>, api: bool, json_output: bool) -> anyhow::Result<()> {
    let effective_backend = if api {
        InferenceBackend::Api
    } else if let Some(backend) = backend {
        backend
    } else if let Some(saved_backend) = state::saved_backend()? {
        saved_backend
    } else {
        vera_core::config::resolve_backend(None)
    };
    let local_embedding_model = embedding_model_for_repair(effective_backend)?;

    setup::repair_backend(
        effective_backend,
        local_embedding_model,
        json_output,
        "Vera repair complete.",
    )
}
