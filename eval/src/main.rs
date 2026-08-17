//! Vera Evaluation Harness
//!
//! Single-command benchmark runner that produces structured JSON results
//! alongside human-readable summaries.
//!
//! Usage:
//!   vera-eval run [--tasks-dir <path>] [--output <path>] [--tool <name>]
//!   vera-eval verify-corpus [--corpus <path>]

mod loader;
mod metrics;
mod output;
mod runner;
mod types;
mod vera_adapter;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "vera-eval", about = "Vera evaluation harness")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the benchmark suite and produce evaluation report.
    Run {
        /// Path to the tasks directory (default: eval/tasks/).
        #[arg(long, default_value = "eval/tasks")]
        tasks_dir: PathBuf,

        /// Path to the corpus manifest (default: eval/corpus.toml).
        #[arg(long, default_value = "eval/corpus.toml")]
        corpus: PathBuf,

        /// Output file path for JSON report (default: stdout).
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Tool adapter to use. `vera-bm25` is the real regression lane; mock tools are for harness self-tests.
        #[arg(long, default_value = "vera-bm25")]
        tool: String,

        /// Suppress human-readable summary (JSON only).
        #[arg(long)]
        json_only: bool,
    },
    /// Verify that corpus repos are cloned at correct SHAs.
    VerifyCorpus {
        /// Path to the corpus manifest.
        #[arg(long, default_value = "eval/corpus.toml")]
        corpus: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            tasks_dir,
            corpus,
            output,
            tool,
            json_only,
        } => cmd_run(&tasks_dir, &corpus, output.as_deref(), &tool, json_only),
        Commands::VerifyCorpus { corpus } => cmd_verify_corpus(&corpus),
    }
}

fn cmd_run(
    tasks_dir: &Path,
    corpus_path: &Path,
    output_path: Option<&Path>,
    tool_name: &str,
    json_only: bool,
) -> Result<()> {
    // Load tasks
    let tasks = loader::load_tasks(tasks_dir)
        .with_context(|| format!("Failed to load tasks from {}", tasks_dir.display()))?;

    if tasks.is_empty() {
        anyhow::bail!("No benchmark tasks found in {}", tasks_dir.display());
    }

    eprintln!("Loaded {} benchmark tasks", tasks.len());

    let report = run_report(tasks, corpus_path, tool_name)?;

    // Output JSON
    if let Some(path) = output_path {
        output::write_json_report(&report, path)?;
        eprintln!("JSON report written to {}", path.display());
    } else if json_only {
        let json = output::report_to_json(&report)?;
        println!("{json}");
    }

    // Print human-readable summary
    if !json_only {
        output::print_summary(&report, &mut std::io::stderr())?;
    }

    Ok(())
}

fn cmd_verify_corpus(corpus_path: &Path) -> Result<()> {
    let manifest = loader::load_corpus(corpus_path)?;
    let repo_root = std::env::current_dir()?;
    let issues = loader::verify_corpus(&manifest, &repo_root)?;

    if issues.is_empty() {
        println!(
            "✓ All {} repos verified at correct SHAs",
            manifest.repos.len()
        );
        for repo in &manifest.repos {
            println!(
                "  {} ({}) → {}",
                repo.name,
                repo.language,
                &repo.commit[..12]
            );
        }
        Ok(())
    } else {
        eprintln!("✗ Corpus verification failed:");
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        eprintln!("\nRun eval/setup-corpus.sh to fix.");
        std::process::exit(1);
    }
}

/// Repo paths, SHAs, and benchmark_root scopes from the corpus manifest.
struct VerifiedCorpus {
    repo_paths: HashMap<String, String>,
    repo_shas: HashMap<String, String>,
    benchmark_roots: HashMap<String, String>,
}

fn load_verified_corpus(corpus_path: &Path) -> Result<VerifiedCorpus> {
    if !corpus_path.exists() {
        anyhow::bail!("Corpus manifest not found at {}", corpus_path.display());
    }

    let manifest = loader::load_corpus(corpus_path)?;
    let repo_root = std::env::current_dir()?;
    let issues = loader::verify_corpus(&manifest, &repo_root)?;
    if !issues.is_empty() {
        anyhow::bail!(
            "Corpus verification failed:\n{}",
            issues
                .into_iter()
                .map(|issue| format!("  - {issue}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let repo_paths = vera_adapter::repo_paths_from_manifest(&repo_root, &manifest);
    let benchmark_roots = vera_adapter::benchmark_roots_from_manifest(&manifest);
    let repo_shas = manifest
        .repos
        .iter()
        .map(|repo| (repo.name.clone(), repo.commit.clone()))
        .collect();

    Ok(VerifiedCorpus {
        repo_paths,
        repo_shas,
        benchmark_roots,
    })
}

/// Filter tasks to only those whose repos are in the corpus manifest.
///
/// When using a subset corpus, tasks for missing repos are silently dropped
/// so the eval harness can run against any corpus subset.
fn filter_tasks_to_corpus(
    tasks: Vec<types::BenchmarkTask>,
    repo_paths: &HashMap<String, String>,
) -> Vec<types::BenchmarkTask> {
    let before = tasks.len();
    let filtered: Vec<_> = tasks
        .into_iter()
        .filter(|task| repo_paths.contains_key(&task.repo))
        .collect();
    let skipped = before - filtered.len();
    if skipped > 0 {
        eprintln!(
            "Skipped {skipped} tasks referencing repos not in corpus ({} tasks remaining)",
            filtered.len()
        );
    }
    filtered
}

fn run_report(
    tasks: Vec<types::BenchmarkTask>,
    corpus_path: &Path,
    tool_name: &str,
) -> Result<types::EvalReport> {
    Ok(match tool_name {
        "mock-perfect" => {
            let mock = runner::MockAdapter::perfect();
            runner::run_benchmark_with_mock(&mock, &tasks)
        }
        "mock-partial" => {
            let mock = runner::MockAdapter::partial(0.7);
            runner::run_benchmark_with_mock(&mock, &tasks)
        }
        "vera-bm25" => {
            let corpus = load_verified_corpus(corpus_path)?;
            let tasks = filter_tasks_to_corpus(tasks, &corpus.repo_paths);
            let vera = vera_adapter::VeraBm25Adapter::new()?;
            runner::run_benchmark_scoped(
                &vera,
                &tasks,
                &corpus.repo_paths,
                &corpus.repo_shas,
                &corpus.benchmark_roots,
            )
        }
        "vera-cuda" => {
            let corpus = load_verified_corpus(corpus_path)?;
            let tasks = filter_tasks_to_corpus(tasks, &corpus.repo_paths);
            let backend = vera_core::config::InferenceBackend::OnnxJina(
                vera_core::config::OnnxExecutionProvider::Cuda,
            );
            let vera = vera_adapter::VeraFullAdapter::new(backend)?;
            runner::run_benchmark_scoped(
                &vera,
                &tasks,
                &corpus.repo_paths,
                &corpus.repo_shas,
                &corpus.benchmark_roots,
            )
        }
        "vera-cpu" => {
            let corpus = load_verified_corpus(corpus_path)?;
            let tasks = filter_tasks_to_corpus(tasks, &corpus.repo_paths);
            let backend = vera_core::config::InferenceBackend::OnnxJina(
                vera_core::config::OnnxExecutionProvider::Cpu,
            );
            let vera = vera_adapter::VeraFullAdapter::new(backend)?;
            runner::run_benchmark_scoped(
                &vera,
                &tasks,
                &corpus.repo_paths,
                &corpus.repo_shas,
                &corpus.benchmark_roots,
            )
        }
        "vera-potion" => {
            let corpus = load_verified_corpus(corpus_path)?;
            let tasks = filter_tasks_to_corpus(tasks, &corpus.repo_paths);
            let vera = vera_adapter::VeraFullAdapter::new(
                vera_core::config::InferenceBackend::PotionCode,
            )?;
            runner::run_benchmark_scoped(
                &vera,
                &tasks,
                &corpus.repo_paths,
                &corpus.repo_shas,
                &corpus.benchmark_roots,
            )
        }
        other => {
            anyhow::bail!(
                "Unknown tool '{}'. Available: vera-bm25, vera-cuda, vera-cpu, vera-potion, mock-perfect, mock-partial.",
                other
            );
        }
    })
}


