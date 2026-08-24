//! Metrics computation library for the Vera evaluation harness.
//!
//! Computes retrieval quality metrics (Recall@k, MRR, nDCG) and
//! performance metrics (latency percentiles, index time, storage, tokens).

use crate::types::{
    AggregateMetrics, BenchmarkTask, GroundTruthEntry, PerformanceMetrics, RetrievalMetrics,
    RetrievalResult, TaskEvaluation, TaskResult,
};

/// Check if a retrieval result matches a ground truth entry.
///
/// A match occurs when the result's file path matches and the line ranges
/// overlap (the result covers at least part of the ground truth range).
fn path_matches(result_path: &str, gt_path: &str) -> bool {
    result_path == gt_path
        || result_path
            .strip_suffix(gt_path)
            .is_some_and(|prefix| prefix.ends_with('/'))
        || gt_path
            .strip_suffix(result_path)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn is_match(result: &RetrievalResult, gt: &GroundTruthEntry) -> bool {
    path_matches(&result.file_path, &gt.file_path)
        && result.line_start <= gt.line_end
        && result.line_end >= gt.line_start
}

fn best_unmatched_ground_truth(
    result: &RetrievalResult,
    ground_truth: &[GroundTruthEntry],
    used: &[bool],
) -> Option<usize> {
    ground_truth
        .iter()
        .enumerate()
        .filter(|(idx, gt)| !used[*idx] && is_match(result, gt))
        .max_by_key(|(_, gt)| gt.relevance)
        .map(|(idx, _)| idx)
}

/// Compute Recall@k: fraction of ground truth items found in top-k results.
pub fn recall_at_k(
    results: &[RetrievalResult],
    ground_truth: &[GroundTruthEntry],
    k: usize,
) -> f64 {
    if ground_truth.is_empty() {
        return 0.0;
    }
    let top_k = &results[..results.len().min(k)];
    let found = ground_truth
        .iter()
        .filter(|gt| top_k.iter().any(|r| is_match(r, gt)))
        .count();
    found as f64 / ground_truth.len() as f64
}

/// Compute Mean Reciprocal Rank (MRR).
///
/// MRR = 1/rank of the first relevant result. Returns 0 if no relevant
/// result is found.
pub fn mrr(results: &[RetrievalResult], ground_truth: &[GroundTruthEntry]) -> f64 {
    for (i, result) in results.iter().enumerate() {
        if ground_truth.iter().any(|gt| is_match(result, gt)) {
            return 1.0 / (i + 1) as f64;
        }
    }
    0.0
}

/// Compute normalized Discounted Cumulative Gain (nDCG).
///
/// Uses the standard formulation:
/// - DCG = sum(relevance_i / log2(i + 2)) for i in 0..k
/// - nDCG = DCG / ideal_DCG
///
/// Relevance is binary (1 if result matches any ground truth, 0 otherwise),
/// weighted by the ground truth entry's relevance score.
pub fn ndcg(results: &[RetrievalResult], ground_truth: &[GroundTruthEntry], k: usize) -> f64 {
    let top_k = &results[..results.len().min(k)];
    let mut used = vec![false; ground_truth.len()];

    // Compute DCG by assigning each result to at most one unmatched ground-truth
    // target. This prevents multiple overlapping chunks from repeatedly
    // claiming credit for the same relevant region.
    let mut dcg = 0.0;
    for (i, result) in top_k.iter().enumerate() {
        if let Some(gt_idx) = best_unmatched_ground_truth(result, ground_truth, &used) {
            used[gt_idx] = true;
            dcg += ground_truth[gt_idx].relevance as f64 / (i as f64 + 2.0).log2();
        }
    }

    // Compute ideal DCG: sort ground truth by relevance descending
    let mut ideal_rels: Vec<f64> = ground_truth.iter().map(|gt| gt.relevance as f64).collect();
    ideal_rels.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let ideal_rels = &ideal_rels[..ideal_rels.len().min(k)];

    let ideal_dcg: f64 = ideal_rels
        .iter()
        .enumerate()
        .map(|(i, &rel)| rel / (i as f64 + 2.0).log2())
        .sum();

    if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg / ideal_dcg
    }
}

/// Compute all retrieval quality metrics for a single task.
pub fn compute_retrieval_metrics(
    results: &[RetrievalResult],
    ground_truth: &[GroundTruthEntry],
) -> RetrievalMetrics {
    RetrievalMetrics {
        recall_at_1: recall_at_k(results, ground_truth, 1),
        recall_at_5: recall_at_k(results, ground_truth, 5),
        recall_at_10: recall_at_k(results, ground_truth, 10),
        mrr: mrr(results, ground_truth),
        ndcg: ndcg(results, ground_truth, 10),
    }
}

/// Compute a percentile from a sorted slice of values.
///
/// Uses linear interpolation between nearest ranks.
pub fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let rank = p / 100.0 * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let frac = rank - lower as f64;

    if lower == upper {
        sorted_values[lower]
    } else {
        sorted_values[lower] * (1.0 - frac) + sorted_values[upper] * frac
    }
}

/// Estimate token count for a string.
///
/// Uses a simple heuristic: ~4 characters per token (standard for English/code).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as f64 / 4.0).ceil() as u64
}

/// Compute performance metrics from task results and indexing data.
pub fn compute_performance_metrics(
    task_results: &[TaskResult],
    index_time_secs: f64,
    storage_size_bytes: u64,
) -> PerformanceMetrics {
    let mut latencies: Vec<f64> = task_results.iter().map(|r| r.latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let total_token_count: u64 = task_results
        .iter()
        .flat_map(|r| &r.results)
        .map(|r| {
            // Estimate tokens from the result content representation
            let content = format!(
                "{}:{}:{} score={}",
                r.file_path, r.line_start, r.line_end, r.score
            );
            estimate_tokens(&content)
        })
        .sum();

    PerformanceMetrics {
        latency_p50_ms: percentile(&latencies, 50.0),
        latency_p95_ms: percentile(&latencies, 95.0),
        index_time_secs,
        storage_size_bytes,
        total_token_count,
    }
}

/// Evaluate all tasks and produce per-task evaluations.
pub fn evaluate_tasks(tasks: &[BenchmarkTask], results: &[TaskResult]) -> Vec<TaskEvaluation> {
    tasks
        .iter()
        .filter_map(|task| {
            let task_result = results.iter().find(|r| r.task_id == task.id)?;
            let retrieval_metrics =
                compute_retrieval_metrics(&task_result.results, &task.ground_truth);
            Some(TaskEvaluation {
                task_id: task.id.clone(),
                category: task.category.clone(),
                retrieval_metrics,
                latency_ms: task_result.latency_ms,
                result_count: task_result.results.len(),
                results: task_result.results.clone(),
            })
        })
        .collect()
}

/// Aggregate metrics across a set of task evaluations.
pub fn aggregate_metrics(
    evaluations: &[TaskEvaluation],
    performance: PerformanceMetrics,
) -> AggregateMetrics {
    let count = evaluations.len();
    if count == 0 {
        return AggregateMetrics {
            retrieval: RetrievalMetrics::default(),
            performance,
            task_count: 0,
        };
    }
    let n = count as f64;
    AggregateMetrics {
        retrieval: RetrievalMetrics {
            recall_at_1: evaluations
                .iter()
                .map(|e| e.retrieval_metrics.recall_at_1)
                .sum::<f64>()
                / n,
            recall_at_5: evaluations
                .iter()
                .map(|e| e.retrieval_metrics.recall_at_5)
                .sum::<f64>()
                / n,
            recall_at_10: evaluations
                .iter()
                .map(|e| e.retrieval_metrics.recall_at_10)
                .sum::<f64>()
                / n,
            mrr: evaluations
                .iter()
                .map(|e| e.retrieval_metrics.mrr)
                .sum::<f64>()
                / n,
            ndcg: evaluations
                .iter()
                .map(|e| e.retrieval_metrics.ndcg)
                .sum::<f64>()
                / n,
        },
        performance,
        task_count: count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(file: &str, start: usize, end: usize) -> RetrievalResult {
        RetrievalResult {
            file_path: file.to_string(),
            line_start: start,
            line_end: end,
            score: 0.0,
        }
    }

    fn make_gt(file: &str, start: usize, end: usize) -> GroundTruthEntry {
        GroundTruthEntry {
            file_path: file.to_string(),
            line_start: start,
            line_end: end,
            relevance: 1,
        }
    }

    #[test]
    fn test_recall_at_k_perfect() {
        let results = vec![make_result("a.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(recall_at_k(&results, &gt, 1), 1.0);
        assert_eq!(recall_at_k(&results, &gt, 5), 1.0);
    }

    #[test]
    fn test_recall_at_k_miss() {
        let results = vec![make_result("b.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(recall_at_k(&results, &gt, 1), 0.0);
    }

    #[test]
    fn test_path_suffix_matching_respects_boundaries() {
        assert!(path_matches("a/util.py", "a/util.py"));
        assert!(path_matches("b/a/util.py", "a/util.py"));
        assert!(path_matches("a/util.py", "b/a/util.py"));
        assert!(!path_matches("store.rs", "mystore.rs"));
    }

    #[test]
    fn test_recall_at_k_partial() {
        let results = vec![make_result("b.rs", 1, 10), make_result("a.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10), make_gt("c.rs", 1, 10)];
        assert_eq!(recall_at_k(&results, &gt, 1), 0.0); // a.rs is rank 2
        assert_eq!(recall_at_k(&results, &gt, 5), 0.5); // found 1 of 2
    }

    #[test]
    fn test_recall_overlap_matching() {
        // Result overlaps with ground truth
        let results = vec![make_result("a.rs", 5, 15)];
        let gt = vec![make_gt("a.rs", 10, 20)];
        assert_eq!(recall_at_k(&results, &gt, 1), 1.0);

        // No overlap
        let results2 = vec![make_result("a.rs", 1, 5)];
        let gt2 = vec![make_gt("a.rs", 10, 20)];
        assert_eq!(recall_at_k(&results2, &gt2, 1), 0.0);
    }

    #[test]
    fn test_recall_empty_ground_truth() {
        let results = vec![make_result("a.rs", 1, 10)];
        let gt: Vec<GroundTruthEntry> = vec![];
        assert_eq!(recall_at_k(&results, &gt, 1), 0.0);
    }

    #[test]
    fn test_mrr_first_hit() {
        let results = vec![make_result("a.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(mrr(&results, &gt), 1.0);
    }

    #[test]
    fn test_mrr_second_hit() {
        let results = vec![make_result("b.rs", 1, 10), make_result("a.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(mrr(&results, &gt), 0.5);
    }

    #[test]
    fn test_mrr_no_hit() {
        let results = vec![make_result("b.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(mrr(&results, &gt), 0.0);
    }

    #[test]
    fn test_ndcg_perfect() {
        let results = vec![make_result("a.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        let score = ndcg(&results, &gt, 10);
        assert!(
            (score - 1.0).abs() < 1e-10,
            "Perfect nDCG should be 1.0, got {score}"
        );
    }

    #[test]
    fn test_ndcg_no_relevant() {
        let results = vec![make_result("b.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        assert_eq!(ndcg(&results, &gt, 10), 0.0);
    }

    #[test]
    fn test_ndcg_duplicate_hits_do_not_exceed_one() {
        let results = vec![make_result("a.rs", 1, 10), make_result("a.rs", 2, 9)];
        let gt = vec![GroundTruthEntry {
            file_path: "a.rs".to_string(),
            line_start: 1,
            line_end: 10,
            relevance: 3,
        }];

        let score = ndcg(&results, &gt, 10);
        assert!(score <= 1.0, "nDCG must stay <= 1.0, got {score}");
        assert!(
            (score - 1.0).abs() < 1e-10,
            "first matching hit should still score perfectly"
        );
    }

    #[test]
    fn test_percentile_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 50.0), 3.0);
        assert_eq!(percentile(&values, 100.0), 5.0);
    }

    #[test]
    fn test_percentile_interpolation() {
        let values = vec![1.0, 3.0];
        let p50 = percentile(&values, 50.0);
        assert!(
            (p50 - 2.0).abs() < 1e-10,
            "p50 of [1, 3] should be 2.0, got {p50}"
        );
    }

    #[test]
    fn test_percentile_single() {
        let values = vec![42.0];
        assert_eq!(percentile(&values, 50.0), 42.0);
        assert_eq!(percentile(&values, 95.0), 42.0);
    }

    #[test]
    fn test_percentile_empty() {
        let values: Vec<f64> = vec![];
        assert_eq!(percentile(&values, 50.0), 0.0);
    }

    #[test]
    fn test_estimate_tokens() {
        // ~4 chars per token
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_compute_retrieval_metrics() {
        let results = vec![make_result("a.rs", 1, 10), make_result("b.rs", 1, 10)];
        let gt = vec![make_gt("a.rs", 1, 10)];
        let metrics = compute_retrieval_metrics(&results, &gt);
        assert_eq!(metrics.recall_at_1, 1.0);
        assert_eq!(metrics.recall_at_5, 1.0);
        assert_eq!(metrics.mrr, 1.0);
        assert!(metrics.ndcg > 0.0);
    }

    #[test]
    fn test_aggregate_metrics() {
        let evals = vec![
            TaskEvaluation {
                task_id: "t1".to_string(),
                category: crate::types::TaskCategory::SymbolLookup,
                retrieval_metrics: RetrievalMetrics {
                    recall_at_1: 1.0,
                    recall_at_5: 1.0,
                    recall_at_10: 1.0,
                    mrr: 1.0,
                    ndcg: 1.0,
                },
                latency_ms: 10.0,
                result_count: 5,
                results: Vec::new(),
            },
            TaskEvaluation {
                task_id: "t2".to_string(),
                category: crate::types::TaskCategory::Intent,
                retrieval_metrics: RetrievalMetrics {
                    recall_at_1: 0.0,
                    recall_at_5: 0.5,
                    recall_at_10: 1.0,
                    mrr: 0.5,
                    ndcg: 0.5,
                },
                latency_ms: 20.0,
                result_count: 10,
                results: Vec::new(),
            },
        ];
        let perf = PerformanceMetrics {
            latency_p50_ms: 15.0,
            latency_p95_ms: 19.0,
            index_time_secs: 5.0,
            storage_size_bytes: 1000,
            total_token_count: 100,
        };
        let agg = aggregate_metrics(&evals, perf);
        assert_eq!(agg.task_count, 2);
        assert!((agg.retrieval.recall_at_1 - 0.5).abs() < 1e-10);
        assert!((agg.retrieval.recall_at_5 - 0.75).abs() < 1e-10);
        assert!((agg.retrieval.mrr - 0.75).abs() < 1e-10);
    }
}
