//! Indexing pipeline orchestrator.
//!
//! Coordinates file discovery, parsing, chunking, embedding, and storage
//! into a single `index_repository` entry point. Produces an [`IndexSummary`]
//! describing the work performed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use tracing::{debug, info, warn};

use crate::CancellationToken;
use crate::config::VeraConfig;
use crate::discovery::{self, DiscoveryResult};
use crate::embedding::{
    EmbeddingError, EmbeddingProvider, embed_chunks_concurrent_with_progress_and_cancellation,
};
use crate::indexing::update::{content_hash, detect_language_for_path};
use crate::parsing;
use crate::parsing::references::RawReference;
use crate::parsing::type_relations::RawTypeRelation;
use crate::storage::bm25::Bm25Index;
use crate::storage::metadata::{FileIndexState, FileIndexStatus, MetadataStore};
use crate::storage::vector::VectorStore;
use crate::types::{Chunk, Language};

// ── Index summary ────────────────────────────────────────────────────

/// Summary of an indexing run, suitable for display to the user.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexSummary {
    /// Number of source files parsed.
    pub files_parsed: usize,
    /// Number of chunks created from parsed files.
    pub chunks_created: usize,
    /// Number of embedding vectors generated.
    pub embeddings_generated: usize,
    /// Number of binary files skipped.
    pub binary_skipped: usize,
    /// Number of files skipped due to size threshold.
    pub large_skipped: usize,
    /// Relative paths and sizes (bytes) of files skipped due to size threshold.
    pub large_skipped_paths: Vec<(String, u64)>,
    /// Number of files skipped due to permission or read errors.
    pub error_skipped: usize,
    /// Number of successfully indexed files whose parse trees contained errors.
    pub files_with_tree_sitter_errors: usize,
    /// Number of successfully indexed files that fell back to Tier 0 chunking.
    pub files_using_tier0_fallback: usize,
    /// Files that had parse errors (path + error message).
    pub parse_errors: Vec<FileError>,
    /// Wall-clock elapsed time in seconds.
    pub elapsed_secs: f64,
}

/// A file-level error encountered during indexing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileError {
    pub file_path: String,
    pub error: String,
}

// ── Progress reporting ───────────────────────────────────────────────

/// Progress events emitted during indexing.
#[derive(Debug, Clone)]
pub enum IndexProgress {
    /// File discovery complete.
    DiscoveryDone { file_count: usize },
    /// Parsing and chunking complete.
    ParsingDone { chunk_count: usize },
    /// An embedding batch finished. `done` is cumulative chunks embedded so far.
    EmbeddingProgress { done: usize, total: usize },
    /// All embeddings generated.
    EmbeddingDone { count: usize },
    /// Index artifacts written to disk.
    StorageDone,
}

// ── Index directory layout ───────────────────────────────────────────

/// Default index directory name (placed inside the indexed repo).
pub(crate) const INDEX_DIR_NAME: &str = ".vera";

/// Subdirectory for BM25 (Tantivy) index files.
const BM25_SUBDIR: &str = "bm25";

/// Filename for SQLite metadata + vector databases.
const METADATA_DB: &str = "metadata.db";
const VECTOR_DB: &str = "vectors.db";

/// Maximum number of parsed chunks held by the full-index pipeline at once.
///
/// The actual bound is approximate when a parse group or a single file
/// produces more chunks than the target. With the default embedding byte
/// limit, this bounds live chunk text to roughly `target * max_chunk_bytes`.
const WINDOW_CHUNK_TARGET: usize = 2048;

/// Keep each rayon parse group bounded while a window is being assembled.
const MAX_PARSE_FILE_GROUP: usize = 64;

/// Resolve the index directory for a given repository root.
pub fn index_dir(repo_root: &Path) -> std::path::PathBuf {
    repo_root.join(INDEX_DIR_NAME)
}

/// True when `path` lives inside the live index directory or one of the
/// staging siblings (`.vera.build`, `.vera.old`) a build swaps in and out.
/// Watchers and file discovery must treat all three as internal artifacts:
/// reacting to staging writes re-triggers watchers, and indexing them
/// duplicates index content as source.
pub fn path_in_index_artifacts(idx_dir: &Path, path: &Path) -> bool {
    path.starts_with(idx_dir)
        || path.starts_with(sibling_index_dir(idx_dir, "build"))
        || path.starts_with(sibling_index_dir(idx_dir, "old"))
}

// ── Pipeline entry point ─────────────────────────────────────────────

/// Index a repository: discover files, parse, chunk, embed, and store.
///
/// This is the main orchestrator for `vera index <path>`. It:
/// 1. Validates the input path
/// 2. Discovers source files (respecting .gitignore and exclusions)
/// 3. Parses and chunks each file
/// 4. Generates embeddings via the provider
/// 5. Stores metadata, vectors, and BM25 index on disk
///
/// # Arguments
/// - `repo_path` — Path to the repository to index
/// - `provider` — Embedding provider (API-backed or mock)
/// - `config` — Pipeline configuration
///
/// # Errors
/// Returns an error if the path is invalid, not a directory, or storage fails.
pub async fn index_repository<P: EmbeddingProvider>(
    repo_path: &Path,
    provider: &P,
    config: &VeraConfig,
    model_name: &str,
) -> Result<IndexSummary> {
    index_repository_with_cancellation(
        repo_path,
        provider,
        config,
        model_name,
        &CancellationToken::new(),
    )
    .await
}

/// Index a repository while cooperatively observing cancellation.
pub async fn index_repository_with_cancellation<P: EmbeddingProvider>(
    repo_path: &Path,
    provider: &P,
    config: &VeraConfig,
    model_name: &str,
    cancellation: &CancellationToken,
) -> Result<IndexSummary> {
    index_repository_with_progress_and_cancellation(
        repo_path,
        provider,
        config,
        model_name,
        |_| {},
        cancellation,
    )
    .await
}

/// Index a repository and report progress to the supplied callback.
pub async fn index_repository_with_progress<P, F>(
    repo_path: &Path,
    provider: &P,
    config: &VeraConfig,
    model_name: &str,
    on_progress: F,
) -> Result<IndexSummary>
where
    P: EmbeddingProvider,
    F: Fn(IndexProgress) + Send + Sync,
{
    index_repository_with_progress_and_cancellation(
        repo_path,
        provider,
        config,
        model_name,
        on_progress,
        &CancellationToken::new(),
    )
    .await
}

/// Index a repository with progress reporting and cooperative cancellation.
pub async fn index_repository_with_progress_and_cancellation<P, F>(
    repo_path: &Path,
    provider: &P,
    config: &VeraConfig,
    model_name: &str,
    on_progress: F,
    cancellation: &CancellationToken,
) -> Result<IndexSummary>
where
    P: EmbeddingProvider,
    F: Fn(IndexProgress) + Send + Sync,
{
    index_repository_with_progress_and_cancellation_with_window_target(
        repo_path,
        provider,
        config,
        model_name,
        on_progress,
        cancellation,
        WINDOW_CHUNK_TARGET,
    )
    .await
}

/// Test seam for running the full-index pipeline with a smaller chunk window.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn index_repository_with_progress_and_cancellation_with_window_target<P, F>(
    repo_path: &Path,
    provider: &P,
    config: &VeraConfig,
    model_name: &str,
    on_progress: F,
    cancellation: &CancellationToken,
    window_chunk_target: usize,
) -> Result<IndexSummary>
where
    P: EmbeddingProvider,
    F: Fn(IndexProgress) + Send + Sync,
{
    let start = Instant::now();
    cancellation.check()?;
    let window_chunk_target = window_chunk_target.max(1);

    // ── 1. Validate path ─────────────────────────────────────────
    if !repo_path.exists() {
        bail!("path does not exist: {}", repo_path.display());
    }
    if !repo_path.is_dir() {
        bail!("path is not a directory: {}", repo_path.display());
    }

    let repo_root = repo_path
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {}", repo_path.display()))?;

    info!(path = %repo_root.display(), "starting indexing");

    let idx_dir = index_dir(&repo_root);
    recover_index_directories(&idx_dir).context("failed to recover index directories")?;

    // ── 2. Discover files ────────────────────────────────────────
    let discovery =
        discovery::discover_files_with_cancellation(&repo_root, &config.indexing, cancellation)
            .context("file discovery failed")?;

    if discovery.files.is_empty() {
        return Ok(IndexSummary {
            files_parsed: 0,
            chunks_created: 0,
            embeddings_generated: 0,
            binary_skipped: discovery.binary_skipped,
            large_skipped: discovery.large_skipped,
            large_skipped_paths: discovery.large_skipped_paths.clone(),
            error_skipped: discovery.error_skipped,
            files_with_tree_sitter_errors: 0,
            files_using_tier0_fallback: 0,
            parse_errors: Vec::new(),
            elapsed_secs: start.elapsed().as_secs_f64(),
        });
    }

    info!(
        files = discovery.files.len(),
        binary_skipped = discovery.binary_skipped,
        large_skipped = discovery.large_skipped,
        error_skipped = discovery.error_skipped,
        "file discovery complete"
    );
    on_progress(IndexProgress::DiscoveryDone {
        file_count: discovery.files.len(),
    });

    // Build into a sibling directory. The guard removes it on every error or
    // cancellation, leaving the previous live index untouched until swap.
    let mut staging = StagingIndex::new(&idx_dir).context("failed to create staging index")?;
    let metadata_store = MetadataStore::open(&staging.build_dir.join(METADATA_DB))
        .context("failed to open staging metadata store")?;
    metadata_store
        .set_index_meta("model_name", model_name)
        .context("failed to store model_name")?;
    let document_prefix = provider.document_prefix_identity();
    metadata_store
        .set_index_meta("document_prefix", &document_prefix)
        .context("failed to store document_prefix")?;

    let bm25_index = Bm25Index::open(&staging.build_dir.join(BM25_SUBDIR))
        .context("failed to open staging BM25 index")?;
    let mut vector_store = None;
    let mut stored_dim = None;

    // ── 3. Parse, embed, and store bounded windows ───────────────
    let (batch_size, max_concurrent_requests) = config.embedding.bounded_parallelism();
    if batch_size != config.embedding.batch_size
        || max_concurrent_requests != config.embedding.max_concurrent_requests
    {
        info!(
            configured_batch_size = config.embedding.batch_size,
            configured_concurrency = config.embedding.max_concurrent_requests,
            max_in_flight_inputs = config.embedding.max_in_flight_inputs,
            batch_size,
            max_concurrent_requests,
            "clamped embedding parallelism to the in-flight input bound"
        );
    }

    let parse_group_size = window_chunk_target.min(MAX_PARSE_FILE_GROUP);
    let mut next_file_index = 0;
    let mut parsed_chunk_count = 0;
    let mut embedded_count = 0;
    let mut parse_errors = Vec::new();
    let mut file_hashes = Vec::new();
    let mut file_states = Vec::new();

    while next_file_index < discovery.files.len() {
        cancellation.check()?;

        let window_start = next_file_index;
        let mut window_chunks = Vec::new();
        let mut window_parse_errors = Vec::new();
        let mut window_file_hashes = Vec::new();
        let mut window_refs = Vec::new();
        let mut window_type_relations = Vec::new();
        let mut window_file_states = Vec::new();

        while next_file_index < discovery.files.len()
            && (next_file_index == window_start || window_chunks.len() < window_chunk_target)
        {
            let group_end = (next_file_index + parse_group_size).min(discovery.files.len());
            let (
                chunks,
                group_parse_errors,
                group_file_hashes,
                group_refs,
                group_type_relations,
                group_file_states,
            ) = parse_discovered_files_parallel(
                &discovery,
                &discovery.files[next_file_index..group_end],
                &repo_root,
                config,
                cancellation,
            )?;
            next_file_index = group_end;
            parsed_chunk_count += chunks.len();
            window_chunks.extend(chunks);
            window_parse_errors.extend(group_parse_errors);
            window_file_hashes.extend(group_file_hashes);
            window_refs.extend(group_refs);
            window_type_relations.extend(group_type_relations);
            window_file_states.extend(group_file_states);
        }

        let is_final_window = next_file_index == discovery.files.len();
        if is_final_window {
            info!(
                chunks = parsed_chunk_count,
                parse_errors = window_parse_errors.len() + parse_errors.len(),
                "parsing complete"
            );
            on_progress(IndexProgress::ParsingDone {
                chunk_count: parsed_chunk_count,
            });
        }

        parse_errors.extend(window_parse_errors);
        file_hashes.extend(window_file_hashes);
        file_states.extend(window_file_states.iter().cloned());

        cancellation.check()?;
        if !window_chunks.is_empty() {
            let embedded_before_window = embedded_count;
            let parsed_through_window = parsed_chunk_count;
            let window_batch_size = batch_size.min(window_chunk_target);
            let progress_cb = |done: usize, _total: usize| {
                on_progress(IndexProgress::EmbeddingProgress {
                    done: embedded_before_window + done,
                    total: parsed_through_window,
                });
            };
            let embedding_result = embed_chunks_concurrent_with_progress_and_cancellation(
                provider,
                &window_chunks,
                window_batch_size,
                max_concurrent_requests,
                config.indexing.max_chunk_bytes,
                cancellation.as_async_token(),
                progress_cb,
            )
            .await;
            let mut embeddings = match embedding_result {
                Ok(embeddings) => embeddings,
                Err(error) => {
                    // A completed provider error outranks a simultaneous cancellation,
                    // mirroring the biased select in the CLI's cancel_task_on_signal.
                    if matches!(error, EmbeddingError::Cancelled) {
                        cancellation.check()?;
                    }
                    return Err(error).context("embedding generation failed");
                }
            };
            cancellation.check()?;

            let window_dim =
                super::truncate_embeddings(&mut embeddings, config.embedding.max_stored_dim);
            if !embeddings.is_empty() {
                if let Some(existing_dim) = stored_dim {
                    anyhow::ensure!(
                        existing_dim == window_dim,
                        "embedding dimension changed between windows: expected {}, got {}",
                        existing_dim,
                        window_dim
                    );
                } else {
                    let store = VectorStore::open(&staging.build_dir.join(VECTOR_DB), window_dim)
                        .context("failed to open staging vector store")?;
                    metadata_store
                        .set_index_meta("embedding_dim", &window_dim.to_string())
                        .context("failed to store embedding_dim")?;
                    vector_store = Some(store);
                    stored_dim = Some(window_dim);
                }
            }
            embedded_count += embeddings.len();

            cancellation.check()?;
            metadata_store
                .insert_chunks(&window_chunks)
                .context("failed to insert chunk metadata")?;
            metadata_store
                .insert_file_states(&window_file_states)
                .context("failed to store file index states")?;
            metadata_store
                .insert_parse_artifacts_batch(&window_refs, &window_type_relations)
                .context("failed to store references and type relations")?;

            if let Some(vector_store) = vector_store.as_ref() {
                let batch: Vec<(&str, &[f32])> = embeddings
                    .iter()
                    .map(|(id, vector)| (id.as_str(), vector.as_slice()))
                    .collect();
                vector_store
                    .insert_batch(&batch)
                    .context("failed to insert vectors")?;
            }
            bm25_index
                .insert_chunks(&window_chunks)
                .context("failed to insert BM25 documents")?;
        } else {
            // Parse-error-only windows still need their file state and parse
            // artifacts persisted if a later window contains chunks.
            metadata_store
                .insert_file_states(&window_file_states)
                .context("failed to store file index states")?;
            metadata_store
                .insert_parse_artifacts_batch(&window_refs, &window_type_relations)
                .context("failed to store references and type relations")?;
        }

        cancellation.check()?;
    }

    if parsed_chunk_count == 0 {
        return Ok(IndexSummary {
            files_parsed: discovery.files.len() - parse_errors.len(),
            chunks_created: 0,
            embeddings_generated: 0,
            binary_skipped: discovery.binary_skipped,
            large_skipped: discovery.large_skipped,
            large_skipped_paths: discovery.large_skipped_paths.clone(),
            error_skipped: discovery.error_skipped,
            files_with_tree_sitter_errors: count_tree_sitter_error_files(&file_states),
            files_using_tier0_fallback: count_tier0_fallback_files(&file_states),
            parse_errors,
            elapsed_secs: start.elapsed().as_secs_f64(),
        });
    }

    on_progress(IndexProgress::EmbeddingDone {
        count: embedded_count,
    });

    // A provider returning no vectors is not expected, but preserve the old
    // empty-vector dimensionality fallback for a non-empty parsed corpus.
    if stored_dim.is_none() {
        let fallback_dim = 4096;
        let store = VectorStore::open(&staging.build_dir.join(VECTOR_DB), fallback_dim)
            .context("failed to open staging vector store")?;
        metadata_store
            .set_index_meta("embedding_dim", &fallback_dim.to_string())
            .context("failed to store embedding_dim")?;
        vector_store = Some(store);
    }

    // Publication is synchronous, so cancellation must win before any artifact is replaced.
    cancellation.check()?;
    publish_index_certification(&metadata_store, &file_hashes, &config.indexing)
        .context("failed to publish index freshness metadata")?;

    let files_parsed = discovery.files.len() - parse_errors.len();
    let summary = IndexSummary {
        files_parsed,
        chunks_created: parsed_chunk_count,
        embeddings_generated: embedded_count,
        binary_skipped: discovery.binary_skipped,
        large_skipped: discovery.large_skipped,
        large_skipped_paths: discovery.large_skipped_paths,
        error_skipped: discovery.error_skipped,
        files_with_tree_sitter_errors: count_tree_sitter_error_files(&file_states),
        files_using_tier0_fallback: count_tier0_fallback_files(&file_states),
        parse_errors,
        elapsed_secs: start.elapsed().as_secs_f64(),
    };

    drop(vector_store);
    drop(bm25_index);
    drop(metadata_store);
    swap_staging_index(&idx_dir, &staging.build_dir, &staging.old_dir)
        .context("failed to publish staged index")?;
    staging.committed = true;

    info!(index_dir = %idx_dir.display(), "index artifacts written");
    on_progress(IndexProgress::StorageDone);

    Ok(summary)
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Parse all discovered files in parallel using rayon and collect chunks.
///
/// Each file is read and parsed on a rayon thread pool worker. Results
/// are collected and flattened. Files that fail parsing are recorded as
/// errors but do not abort the pipeline. Also computes content hashes
/// for incremental indexing support.
#[allow(clippy::type_complexity)]
fn parse_discovered_files_parallel(
    discovery: &DiscoveryResult,
    files: &[discovery::DiscoveredFile],
    repo_root: &Path,
    config: &VeraConfig,
    cancellation: &CancellationToken,
) -> Result<(
    Vec<Chunk>,
    Vec<FileError>,
    Vec<(String, String)>,
    Vec<(String, Vec<RawReference>)>,
    Vec<(String, Vec<RawTypeRelation>)>,
    Vec<FileIndexState>,
)> {
    let config = Arc::new(config.clone());
    let repo_root = Arc::new(repo_root.to_path_buf());

    struct ParsedFileResult {
        chunks: Vec<Chunk>,
        parse_error: Option<FileError>,
        file_hash: Option<(String, String)>,
        refs: Option<(String, Vec<RawReference>)>,
        type_relations: Option<(String, Vec<RawTypeRelation>)>,
        file_state: Option<FileIndexState>,
    }

    let results: Vec<ParsedFileResult> = files
        .par_iter()
        .map(|file| {
            if cancellation.is_cancelled() {
                return ParsedFileResult {
                    chunks: Vec::new(),
                    parse_error: None,
                    file_hash: None,
                    refs: None,
                    type_relations: None,
                    file_state: None,
                };
            }

            let source = match crate::discovery::read_source_lossy_at(
                &discovery.root_dir,
                Path::new(&file.relative_path),
            ) {
                Ok(source) => source,
                Err(err) => {
                    warn!(
                        file = %file.relative_path,
                        error = %err,
                        "failed to read file for parsing"
                    );
                    return ParsedFileResult {
                        chunks: Vec::new(),
                        parse_error: Some(FileError {
                            file_path: file.relative_path.clone(),
                            error: err.to_string(),
                        }),
                        file_hash: None,
                        refs: None,
                        type_relations: None,
                        file_state: None,
                    };
                }
            };

            let language = detect_language_for_path(&file.absolute_path);

            // RST files need preprocessing before chunking, but refs
            // come from the raw source, so they can't share a single parse.
            // The hash is computed before the parse so the error branch can
            // store the hash of the source that was actually attempted;
            // otherwise a parse-failing RST file looks modified on every
            // update and is re-parsed forever.
            let hash;
            let parsed = if language == Language::Rst {
                let refs = parsing::parse_and_extract_references(&source, language);
                let normalized_source = match parsing::sphinx::preprocess_rst_with_limit(
                    &source,
                    &file.absolute_path,
                    repo_root.as_path(),
                    config.indexing.max_file_size_bytes,
                ) {
                    Ok(preprocessed) => Some(preprocessed),
                    Err(err) => {
                        warn!(
                            file = %file.relative_path,
                            error = %err,
                            "failed to preprocess rst; falling back to raw source"
                        );
                        None
                    }
                };
                let src = normalized_source.as_deref().unwrap_or(&source);
                hash = content_hash(src);
                parsing::parse_file_with_diagnostics(
                    src,
                    &file.relative_path,
                    language,
                    &config.indexing,
                )
                .map(|(chunks, _ignored_refs, diagnostics)| (chunks, refs, diagnostics))
            } else {
                hash = content_hash(&source);
                parsing::parse_file_with_diagnostics(
                    &source,
                    &file.relative_path,
                    language,
                    &config.indexing,
                )
            };

            match parsed {
                Ok((chunks, refs, diagnostics)) => {
                    let chunk_count = chunks.len() as u64;
                    let type_relations = parsing::type_relations::extract_type_relations(&chunks);
                    debug!(
                        file = %file.relative_path,
                        chunks = chunk_count,
                        refs = refs.len(),
                        type_relations = type_relations.len(),
                        "parsed file"
                    );
                    ParsedFileResult {
                        chunks,
                        parse_error: None,
                        file_hash: Some((file.relative_path.clone(), hash)),
                        refs: (!refs.is_empty()).then_some((file.relative_path.clone(), refs)),
                        type_relations: (!type_relations.is_empty())
                            .then_some((file.relative_path.clone(), type_relations)),
                        file_state: Some(FileIndexState {
                            file_path: file.relative_path.clone(),
                            language: language.to_string(),
                            status: FileIndexStatus::Indexed,
                            tree_has_error: diagnostics.tree_has_error,
                            tier0_fallback: diagnostics.used_tier0_fallback,
                            chunk_count,
                        }),
                    }
                }
                Err(err) => {
                    warn!(
                        file = %file.relative_path,
                        error = %err,
                        "parse error"
                    );
                    ParsedFileResult {
                        chunks: Vec::new(),
                        parse_error: Some(FileError {
                            file_path: file.relative_path.clone(),
                            error: err.to_string(),
                        }),
                        file_hash: Some((file.relative_path.clone(), hash)),
                        refs: None,
                        type_relations: None,
                        file_state: Some(FileIndexState {
                            file_path: file.relative_path.clone(),
                            language: language.to_string(),
                            status: FileIndexStatus::ParseError,
                            tree_has_error: false,
                            tier0_fallback: false,
                            chunk_count: 0,
                        }),
                    }
                }
            }
        })
        .collect();

    cancellation.check()?;

    // Flatten results into chunks, errors, file hashes, and references.
    let mut all_chunks = Vec::new();
    let mut parse_errors = Vec::new();
    let mut file_hashes = Vec::new();
    let mut all_refs = Vec::new();
    let mut all_type_relations = Vec::new();
    let mut file_states = Vec::new();
    for result in results {
        all_chunks.extend(result.chunks);
        if let Some(error) = result.parse_error {
            parse_errors.push(error);
        }
        if let Some(file_hash) = result.file_hash {
            file_hashes.push(file_hash);
        }
        if let Some(file_refs) = result.refs {
            all_refs.push(file_refs);
        }
        if let Some(type_relations) = result.type_relations {
            all_type_relations.push(type_relations);
        }
        if let Some(file_state) = result.file_state {
            file_states.push(file_state);
        }
    }

    Ok((
        all_chunks,
        parse_errors,
        file_hashes,
        all_refs,
        all_type_relations,
        file_states,
    ))
}

struct StagingIndex {
    build_dir: PathBuf,
    old_dir: PathBuf,
    committed: bool,
}

impl StagingIndex {
    fn new(idx_dir: &Path) -> Result<Self> {
        let build_dir = sibling_index_dir(idx_dir, "build");
        let old_dir = sibling_index_dir(idx_dir, "old");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir).with_context(|| {
                format!(
                    "failed to remove stale staging dir: {}",
                    build_dir.display()
                )
            })?;
        }
        std::fs::create_dir_all(&build_dir)
            .with_context(|| format!("failed to create staging dir: {}", build_dir.display()))?;
        Ok(Self {
            build_dir,
            old_dir,
            committed: false,
        })
    }
}

impl Drop for StagingIndex {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.build_dir);
        }
    }
}

fn sibling_index_dir(idx_dir: &Path, suffix: &str) -> PathBuf {
    let name = idx_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(INDEX_DIR_NAME);
    idx_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{name}.{suffix}"))
}

fn recover_index_directories(idx_dir: &Path) -> Result<()> {
    let build_dir = sibling_index_dir(idx_dir, "build");
    let old_dir = sibling_index_dir(idx_dir, "old");
    if !idx_dir.exists() && old_dir.exists() {
        std::fs::rename(&old_dir, idx_dir).with_context(|| {
            format!(
                "failed to restore previous index from {}",
                old_dir.display()
            )
        })?;
    }
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir).with_context(|| {
            format!(
                "failed to remove stale staging dir: {}",
                build_dir.display()
            )
        })?;
    }
    if old_dir.exists() {
        std::fs::remove_dir_all(&old_dir)
            .with_context(|| format!("failed to remove stale old index: {}", old_dir.display()))?;
    }
    Ok(())
}

fn swap_staging_index(idx_dir: &Path, build_dir: &Path, old_dir: &Path) -> Result<()> {
    if old_dir.exists() {
        std::fs::remove_dir_all(old_dir)
            .with_context(|| format!("failed to remove old index: {}", old_dir.display()))?;
    }
    if idx_dir.exists() {
        std::fs::rename(idx_dir, old_dir).with_context(|| {
            format!(
                "failed to move live index to temporary path: {}",
                old_dir.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(build_dir, idx_dir) {
        if old_dir.exists() && !idx_dir.exists() {
            let _ = std::fs::rename(old_dir, idx_dir);
        }
        return Err(error)
            .with_context(|| format!("failed to publish staging index as {}", idx_dir.display()));
    }
    if old_dir.exists() {
        std::fs::remove_dir_all(old_dir)
            .with_context(|| format!("failed to remove old index: {}", old_dir.display()))?;
    }
    Ok(())
}

fn publish_index_certification(
    metadata_store: &MetadataStore,
    file_hashes: &[(String, String)],
    indexing_config: &crate::config::IndexingConfig,
) -> Result<()> {
    // Hashes and the freshness stamp certify the index current, so they are
    // written only after every staged metadata, vector, and BM25 insert has
    // completed. A failure before this publication leaves the staging build
    // uncertified and the previous live index untouched. The swap happens
    // only after these writes succeed.
    metadata_store
        .set_file_hashes_batch(file_hashes)
        .context("failed to store file hashes")?;
    super::freshness::record_index_snapshot(metadata_store, indexing_config)
        .context("failed to store index freshness metadata")?;
    Ok(())
}

pub(crate) fn count_tree_sitter_error_files(file_states: &[FileIndexState]) -> usize {
    file_states
        .iter()
        .filter(|state| state.status == FileIndexStatus::Indexed && state.tree_has_error)
        .count()
}

pub(crate) fn count_tier0_fallback_files(file_states: &[FileIndexState]) -> usize {
    file_states
        .iter()
        .filter(|state| state.status == FileIndexStatus::Indexed && state.tier0_fallback)
        .count()
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
