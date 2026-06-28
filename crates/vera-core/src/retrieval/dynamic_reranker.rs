use crate::config::{InferenceBackend, VeraConfig};
use crate::retrieval::local_reranker::LocalReranker;
use crate::retrieval::reranker::{
    ApiReranker, RerankScore, Reranker, RerankerConfig, RerankerError,
};
use anyhow::Result;
use std::time::Duration;

pub enum DynamicReranker {
    Api(ApiReranker),
    Local(LocalReranker),
}

impl Reranker for DynamicReranker {
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
    ) -> Result<Vec<RerankScore>, RerankerError> {
        match self {
            Self::Api(p) => p.rerank(query, documents).await,
            Self::Local(p) => p.rerank(query, documents).await,
        }
    }
}

pub async fn create_dynamic_reranker(
    config: &VeraConfig,
    backend: InferenceBackend,
) -> anyhow::Result<Option<DynamicReranker>> {
    if !config.retrieval.reranking_enabled {
        return Ok(None);
    }

    match backend {
        InferenceBackend::OnnxJina(ep) => {
            // Prefer API reranking when credentials are configured, even when
            // embeddings use a local ONNX backend. This avoids slow or
            // unsupported local reranker execution providers without changing
            // the selected embedding backend.
            if let Some(reranker) = api_reranker_from_env()? {
                return Ok(Some(reranker));
            }

            let p = LocalReranker::new_with_ep(ep)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to initialize local reranker: {e}\nHint: check network connection or manually place model at ~/.vera/models/"))?;
            Ok(Some(DynamicReranker::Local(p)))
        }
        InferenceBackend::Api => api_reranker_from_env(),
    }
}

fn api_reranker_from_env() -> anyhow::Result<Option<DynamicReranker>> {
    let Ok(cfg) = RerankerConfig::from_env() else {
        return Ok(None);
    };

    let cfg = cfg
        .with_timeout(Duration::from_secs(30))
        .with_max_retries(2);
    let p =
        ApiReranker::new(cfg).map_err(|err| anyhow::anyhow!("failed to init reranker: {err}"))?;
    Ok(Some(DynamicReranker::Api(p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OnnxExecutionProvider, RetrievalConfig};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env(key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

    struct EnvSnapshot {
        values: Vec<(&'static str, Option<String>)>,
    }

    impl EnvSnapshot {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                values: keys
                    .iter()
                    .map(|key| (*key, std::env::var(key).ok()))
                    .collect(),
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => set_env(key, value),
                    None => remove_env(key),
                }
            }
        }
    }

    #[tokio::test]
    async fn onnx_backend_prefers_api_reranker_when_env_is_configured() {
        let _guard = env_lock().lock().unwrap();
        let _snapshot = EnvSnapshot::capture(&[
            "RERANKER_MODEL_BASE_URL",
            "RERANKER_MODEL_ID",
            "RERANKER_MODEL_API_KEY",
        ]);

        set_env("RERANKER_MODEL_BASE_URL", "http://127.0.0.1:0/v1");
        set_env("RERANKER_MODEL_ID", "dummy-reranker");
        set_env("RERANKER_MODEL_API_KEY", "dummy-key");

        let mut config = VeraConfig::default();
        config.retrieval = RetrievalConfig {
            reranking_enabled: true,
            ..RetrievalConfig::default()
        };

        let reranker = create_dynamic_reranker(
            &config,
            InferenceBackend::OnnxJina(OnnxExecutionProvider::Cpu),
        )
        .await
        .unwrap();

        assert!(
            matches!(reranker, Some(DynamicReranker::Api(_))),
            "configured API reranker should take precedence over local ONNX reranker"
        );
    }
}
