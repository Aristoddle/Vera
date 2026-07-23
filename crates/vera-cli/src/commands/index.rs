//! `vera index <path>` — Index a codebase for search.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use vera_core::config::InferenceBackend;
use vera_core::indexing::IndexProgress;

use crate::helpers::{load_runtime_config, print_human_summary};

async fn wait_for_interrupt() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn cancel_on_signal<T, Operation, Signal>(
    operation: Operation,
    signal: Signal,
) -> anyhow::Result<T>
where
    Operation: std::future::Future<Output = anyhow::Result<T>>,
    Signal: std::future::Future<Output = ()>,
{
    tokio::select! {
        result = operation => result,
        _ = signal => bail!("indexing cancelled"),
    }
}

/// Run the `vera index <path>` command.
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &str,
    json_output: bool,
    backend: InferenceBackend,
    exclude: Vec<String>,
    no_ignore: bool,
    no_default_excludes: bool,
    verbose: bool,
    low_vram: bool,
) -> anyhow::Result<()> {
    let summary = execute(
        path,
        json_output,
        backend,
        exclude,
        no_ignore,
        no_default_excludes,
        low_vram,
    )?;

    if json_output {
        let json = serde_json::to_string_pretty(&summary)
            .map_err(|e| anyhow::anyhow!("failed to serialize summary: {e}"))?;
        println!("{json}");
    } else {
        print_human_summary(&summary, verbose);
    }

    Ok(())
}

/// Index a repository and return the resulting summary.
pub fn execute(
    path: &str,
    json_output: bool,
    backend: InferenceBackend,
    exclude: Vec<String>,
    no_ignore: bool,
    no_default_excludes: bool,
    low_vram: bool,
) -> anyhow::Result<vera_core::indexing::IndexSummary> {
    let repo_path = Path::new(path);

    if !repo_path.exists() {
        bail!(
            "path does not exist: {path}\n\
             Hint: check the path and try again."
        );
    }
    if !repo_path.is_dir() {
        bail!(
            "path is not a directory: {path}\n\
             Hint: vera index expects a directory path, not a file."
        );
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow::anyhow!("failed to create async runtime: {e}"))?;

    let mut config = load_runtime_config()?;
    if low_vram {
        config.embedding.low_vram = true;
    }
    config.adjust_for_backend(backend);
    config.indexing.extra_excludes = exclude;
    config.indexing.no_ignore = no_ignore;
    config.indexing.no_default_excludes = no_default_excludes;

    let (provider, model_name) = rt.block_on(vera_core::embedding::create_dynamic_provider(
        &config, backend,
    ))?;

    // Use progress bar for interactive (non-JSON) output.
    if json_output {
        let summary = rt
            .block_on(cancel_on_signal(
                vera_core::indexing::index_repository(repo_path, &provider, &config, &model_name),
                wait_for_interrupt(),
            ))
            .context("indexing failed")?;
        return Ok(summary);
    }

    let multi = cliclack::multi_progress("Indexing...");
    let spinner = multi.add(cliclack::spinner());
    spinner.start("Discovering files...");
    let embed_bar: Arc<cliclack::ProgressBar> = Arc::new(multi.add(cliclack::progress_bar(0)));
    let embed_started = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let spinner_ref = &spinner;
    let embed_bar_ref = embed_bar.clone();
    let embed_started_ref = embed_started.clone();

    let on_progress = move |event: IndexProgress| match event {
        IndexProgress::DiscoveryDone { file_count } => {
            spinner_ref.stop(format!("Discovered {file_count} files"));
        }
        IndexProgress::ParsingDone { chunk_count } => {
            spinner_ref.start(format!("Parsed into {chunk_count} chunks"));
            spinner_ref.stop(format!("Parsed into {chunk_count} chunks"));
        }
        IndexProgress::EmbeddingProgress { done, total } => {
            if !embed_started_ref.load(std::sync::atomic::Ordering::Relaxed) {
                embed_started_ref.store(true, std::sync::atomic::Ordering::Relaxed);
                embed_bar_ref.set_length(total as u64);
                embed_bar_ref.start("Generating embeddings...");
            }
            embed_bar_ref.set_position(done as u64);
            embed_bar_ref.set_message(format!("Generating embeddings ({done}/{total})"));
        }
        IndexProgress::EmbeddingDone { count } => {
            embed_bar_ref.stop(format!("Generated {count} embeddings"));
        }
        IndexProgress::StorageDone => {}
    };

    let result = rt.block_on(cancel_on_signal(
        vera_core::indexing::index_repository_with_progress(
            repo_path,
            &provider,
            &config,
            &model_name,
            on_progress,
        ),
        wait_for_interrupt(),
    ));
    multi.stop();

    result.context("indexing failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DropMarker(Arc<AtomicBool>);

    impl Drop for DropMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn operation_result_wins_before_cancellation() {
        let result = cancel_on_signal(async { Ok::<_, anyhow::Error>(42) }, std::future::pending())
            .await
            .unwrap();

        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn cancellation_drops_in_flight_operation() {
        let dropped = Arc::new(AtomicBool::new(false));
        let dropped_for_operation = dropped.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();

        let operation = async move {
            let _marker = DropMarker(dropped_for_operation);
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
            Ok::<_, anyhow::Error>(())
        };
        let signal = async move {
            let _ = started_rx.await;
        };

        let error = cancel_on_signal(operation, signal).await.unwrap_err();

        assert!(error.to_string().contains("cancelled"));
        assert!(dropped.load(Ordering::SeqCst));
    }
}
