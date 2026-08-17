//! Local model asset download, validation, and inspection.

use crate::config::OnnxExecutionProvider;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use super::ort::*;
use super::*;

/// Download a file from HuggingFace Hub using atomic writes.
pub async fn ensure_model_file(repo_id: &str, file_path: &str) -> Result<PathBuf> {
    ensure_model_file_with_kind(repo_id, file_path, asset_kind_for_file(file_path)).await
}

pub(super) async fn ensure_model_file_with_kind(
    repo_id: &str,
    file_path: &str,
    asset_kind: LocalModelAssetKind,
) -> Result<PathBuf> {
    ensure_model_file_impl(repo_id, file_path, asset_kind, HUB_URL, None).await
}

pub fn configured_local_model_name() -> String {
    LocalEmbeddingModelConfig::from_env()
        .map(|config| config.model_identity())
        .unwrap_or_else(|_| EMBEDDING_REPO.to_string())
}

pub fn potion_code_model_dir() -> Result<PathBuf> {
    Ok(vera_home_dir()?.join("models").join(POTION_CODE_REPO))
}

pub fn potion_code_model_name() -> &'static str {
    POTION_CODE_REPO
}

pub async fn ensure_potion_code_assets() -> Result<PathBuf> {
    ensure_model_file_with_kind(
        POTION_CODE_REPO,
        POTION_CODE_TOKENIZER_FILE,
        LocalModelAssetKind::Other,
    )
    .await?;
    ensure_model_file_with_kind(
        POTION_CODE_REPO,
        POTION_CODE_MODEL_FILE,
        LocalModelAssetKind::Other,
    )
    .await?;
    ensure_model_file_with_kind(
        POTION_CODE_REPO,
        POTION_CODE_CONFIG_FILE,
        LocalModelAssetKind::Other,
    )
    .await?;
    potion_code_model_dir()
}

pub fn inspect_potion_code_model_files() -> Result<Vec<LocalModelAssetStatus>> {
    let model_dir = potion_code_model_dir()?;
    let files = [
        ("potion-tokenizer", POTION_CODE_TOKENIZER_FILE),
        ("potion-model", POTION_CODE_MODEL_FILE),
        ("potion-config", POTION_CODE_CONFIG_FILE),
    ];

    Ok(files
        .into_iter()
        .map(|(name, file)| inspect_asset(name, model_dir.join(file), LocalModelAssetKind::Other))
        .collect())
}

pub async fn ensure_local_embedding_assets(
    config: &LocalEmbeddingModelConfig,
) -> Result<LocalEmbeddingAssetPaths> {
    match &config.source {
        LocalEmbeddingSource::HuggingFace { repo } => Ok(LocalEmbeddingAssetPaths {
            onnx_path: ensure_model_file_with_kind(
                repo,
                &config.onnx_file,
                LocalModelAssetKind::Onnx,
            )
            .await?,
            onnx_data_path: match config.onnx_data_file.as_deref() {
                Some(path) => {
                    Some(ensure_model_file_with_kind(repo, path, LocalModelAssetKind::Other).await?)
                }
                None => None,
            },
            tokenizer_path: ensure_model_file_with_kind(
                repo,
                &config.tokenizer_file,
                LocalModelAssetKind::Other,
            )
            .await?,
        }),
        LocalEmbeddingSource::Directory { .. } => verify_local_embedding_assets(config),
    }
}

pub(super) fn verify_local_embedding_assets(
    config: &LocalEmbeddingModelConfig,
) -> Result<LocalEmbeddingAssetPaths> {
    let paths = config.cached_asset_paths()?;
    require_valid_file(
        &paths.onnx_path,
        "embedding ONNX model",
        LocalModelAssetKind::Onnx,
    )?;
    if let Some(path) = paths.onnx_data_path.as_ref() {
        require_valid_file(
            path,
            "embedding ONNX external data",
            LocalModelAssetKind::Other,
        )?;
    }
    require_valid_file(
        &paths.tokenizer_path,
        "embedding tokenizer",
        LocalModelAssetKind::Other,
    )?;
    Ok(paths)
}

pub(super) fn require_valid_file(
    path: &Path,
    label: &str,
    asset_kind: LocalModelAssetKind,
) -> Result<()> {
    if !path.exists() {
        anyhow::bail!(
            "{label} not found at {}.\nHint: place the file there, or point `vera setup` at a Hugging Face repo instead.",
            path.display()
        );
    }
    validate_file(path, asset_kind)
        .with_context(|| format!("{label} integrity check failed at {}", path.display()))
}

/// Download or validate the local embedding model, the curated local reranker, and the ORT library.
pub async fn prepare_local_models_for_ep(
    ep: OnnxExecutionProvider,
    embedding_model: &LocalEmbeddingModelConfig,
) -> Result<Vec<PathBuf>> {
    let mut model = embedding_model.clone();
    model.adjust_for_gpu(ep);
    let mut paths = Vec::new();
    let ort_path = if ep == OnnxExecutionProvider::Cuda {
        refresh_ort_library_for_ep(ep).await?
    } else {
        ensure_ort_library_for_ep(ep).await?
    };
    paths.push(ort_path);
    let embedding_paths = ensure_local_embedding_assets(&model).await?;
    paths.push(embedding_paths.onnx_path);
    if let Some(path) = embedding_paths.onnx_data_path {
        paths.push(path);
    }
    paths.push(embedding_paths.tokenizer_path);
    paths.push(
        ensure_model_file_with_kind(RERANKER_REPO, RERANKER_ONNX_FILE, LocalModelAssetKind::Onnx)
            .await?,
    );
    paths.push(
        ensure_model_file_with_kind(
            RERANKER_REPO,
            RERANKER_TOKENIZER_FILE,
            LocalModelAssetKind::Other,
        )
        .await?,
    );
    Ok(paths)
}

pub fn inspect_local_model_files_for_ep(
    ep: OnnxExecutionProvider,
    embedding_model: &LocalEmbeddingModelConfig,
) -> Result<Vec<LocalModelAssetStatus>> {
    let mut model = embedding_model.clone();
    model.adjust_for_gpu(ep);
    let embedding_paths = model.cached_asset_paths()?;
    let ort_path = ort_library_path_for_ep(ep)?;
    let vera_home = vera_home_dir()?;
    let reranker_onnx = vera_home
        .join("models")
        .join(RERANKER_REPO)
        .join(RERANKER_ONNX_FILE);
    let reranker_tokenizer = vera_home
        .join("models")
        .join(RERANKER_REPO)
        .join(RERANKER_TOKENIZER_FILE);
    let mut assets = vec![
        inspect_asset("onnx-runtime", ort_path, LocalModelAssetKind::Other),
        inspect_asset(
            "embedding-onnx",
            embedding_paths.onnx_path,
            LocalModelAssetKind::Onnx,
        ),
        inspect_asset(
            "embedding-tokenizer",
            embedding_paths.tokenizer_path,
            LocalModelAssetKind::Other,
        ),
        inspect_asset("reranker-onnx", reranker_onnx, LocalModelAssetKind::Onnx),
        inspect_asset(
            "reranker-tokenizer",
            reranker_tokenizer,
            LocalModelAssetKind::Other,
        ),
    ];

    if let Some(path) = embedding_paths.onnx_data_path {
        assets.insert(
            2,
            inspect_asset("embedding-onnx-data", path, LocalModelAssetKind::Other),
        );
    }

    Ok(assets)
}

pub(super) fn inspect_asset(
    name: &'static str,
    path: PathBuf,
    asset_kind: LocalModelAssetKind,
) -> LocalModelAssetStatus {
    let exists = path.exists();
    let (state, detail) = if !exists {
        (
            LocalModelAssetState::Missing,
            Some("file not found".to_string()),
        )
    } else {
        match validate_file(&path, asset_kind) {
            Ok(()) => (LocalModelAssetState::Valid, None),
            Err(error) => (LocalModelAssetState::Invalid, Some(error.to_string())),
        }
    };
    LocalModelAssetStatus {
        name,
        path,
        exists,
        state,
        detail,
    }
}

pub(super) fn validate_file(path: &Path, asset_kind: LocalModelAssetKind) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect file {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("path is not a regular file");
    }
    if metadata.len() == 0 {
        anyhow::bail!("file is empty");
    }
    if asset_kind == LocalModelAssetKind::Onnx {
        validate_onnx_header(path)?;
    }
    Ok(())
}

pub(super) fn validate_onnx_header(path: &Path) -> Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open ONNX file {}", path.display()))?;
    let mut header = Vec::with_capacity(ONNX_HEADER_MAX_BYTES);
    file.take(ONNX_HEADER_MAX_BYTES as u64)
        .read_to_end(&mut header)?;

    let mut offset = 0;
    for _ in 0..ONNX_HEADER_MAX_FIELDS {
        if offset == header.len() {
            break;
        }
        let (key, next_offset) = read_protobuf_varint(&header, offset)
            .context("file does not contain a complete protobuf field header")?;
        offset = next_offset;
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        if field_number == 0 {
            anyhow::bail!("ONNX ModelProto contains an invalid field number");
        }

        match wire_type {
            0 => {
                let (value, next_offset) = read_protobuf_varint(&header, offset)
                    .context("ONNX ModelProto field is truncated")?;
                offset = next_offset;
                if field_number == 1 {
                    if value == 0 {
                        anyhow::bail!("ONNX ModelProto ir_version is zero");
                    }
                    return Ok(());
                }
            }
            1 => offset = skip_protobuf_bytes(&header, offset, 8)?,
            2 => {
                let (length, next_offset) = read_protobuf_varint(&header, offset)
                    .context("ONNX ModelProto length-delimited field is truncated")?;
                let length =
                    usize::try_from(length).context("ONNX ModelProto field length is too large")?;
                offset = skip_protobuf_bytes(&header, next_offset, length)?;
            }
            5 => offset = skip_protobuf_bytes(&header, offset, 4)?,
            3 | 4 => anyhow::bail!("ONNX ModelProto uses unsupported protobuf groups"),
            _ => anyhow::bail!("ONNX ModelProto contains an unknown protobuf wire type"),
        }
    }

    anyhow::bail!(
        "ONNX ModelProto ir_version field was not found in the first {} bytes",
        ONNX_HEADER_MAX_BYTES
    )
}

pub(super) fn skip_protobuf_bytes(bytes: &[u8], offset: usize, length: usize) -> Result<usize> {
    let end = offset
        .checked_add(length)
        .context("ONNX ModelProto field length overflows")?;
    if end > bytes.len() {
        anyhow::bail!("ONNX ModelProto field is truncated");
    }
    Ok(end)
}

pub(super) fn read_protobuf_varint(bytes: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().copied().enumerate().skip(offset) {
        let shift = (index - offset) * 7;
        if shift >= 64 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

pub(super) fn asset_kind_for_file(file_path: &str) -> LocalModelAssetKind {
    if Path::new(file_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"))
    {
        LocalModelAssetKind::Onnx
    } else {
        LocalModelAssetKind::Other
    }
}

pub(super) async fn ensure_model_file_impl(
    repo_id: &str,
    file_path: &str,
    asset_kind: LocalModelAssetKind,
    base_url: &str,
    home_override: Option<&std::path::Path>,
) -> Result<PathBuf> {
    let home_dir = match home_override {
        Some(p) => p.to_path_buf(),
        None => vera_home_dir()?,
    };
    let models_dir = home_dir.join("models").join(repo_id);
    let target_path = models_dir.join(file_path);

    if target_path.exists() {
        match validate_file(&target_path, asset_kind) {
            Ok(()) => return Ok(target_path),
            Err(error) => {
                tracing::warn!(
                    path = %target_path.display(),
                    error = %error,
                    "cached model file failed integrity check; re-downloading"
                );
            }
        }
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let url = format!("{}/{}/resolve/main/{}", base_url, repo_id, file_path);
    eprintln!("Downloading {}...", url);

    crate::init_tls();
    let client = Client::new();
    let res = client.get(&url).send().await?.error_for_status()?;
    let total_size = res.content_length();

    let attempt = MODEL_DOWNLOAD_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temp_path = target_path.with_extension(format!("part.{}.{}", std::process::id(), attempt));
    let mut downloaded = 0;
    let mut temp_created = false;

    let download_result: Result<()> = async {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await?;
        temp_created = true;
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| anyhow::anyhow!("Download error: {}", e))?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if let Some(total) = total_size {
                eprint!(
                    "\rProgress: {} MB / {} MB",
                    downloaded / 1_000_000,
                    total / 1_000_000
                );
            } else {
                eprint!("\rProgress: {} MB", downloaded / 1_000_000);
            }
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        validate_file(&temp_path, asset_kind)
            .with_context(|| format!("downloaded model failed integrity check: {file_path}"))?;
        eprintln!("\nDownload complete: {}", file_path);

        #[cfg(windows)]
        if target_path.exists() {
            fs::remove_file(&target_path).await.with_context(|| {
                format!("failed to replace cached model {}", target_path.display())
            })?;
        }
        fs::rename(&temp_path, &target_path).await?;
        Ok(())
    }
    .await;

    if let Err(e) = download_result {
        if temp_created {
            let _ = fs::remove_file(&temp_path).await;
        }
        return Err(e).context(format!(
            "Expected path: {}. Hint: check network connection or manually place model at {}",
            target_path.display(),
            target_path.display()
        ));
    }

    Ok(target_path)
}
