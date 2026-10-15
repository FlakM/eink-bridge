use crate::metrics;
use std::{
    future::Future,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Recognition {
    pub text: String,
    pub provider: &'static str,
}

struct Observation {
    provider: &'static str,
    started: Instant,
    finished: bool,
}

impl Observation {
    fn finish(&mut self, result: &'static str) {
        metrics::OCR_PROVIDER_REQUESTS
            .with_label_values(&[self.provider, result])
            .inc();
        metrics::OCR_PROVIDER_DURATION
            .with_label_values(&[self.provider, result])
            .observe(self.started.elapsed().as_secs_f64());
        tracing::info!(
            provider = self.provider,
            result,
            wall_ms = self.started.elapsed().as_millis(),
            "OCR provider finished"
        );
        self.finished = true;
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("cancelled");
        }
    }
}

pub(super) async fn observe(
    provider: &'static str,
    future: impl Future<Output = Result<String, String>>,
) -> Result<Recognition, String> {
    let mut observation = Observation {
        provider,
        started: Instant::now(),
        finished: false,
    };
    let result = future.await.and_then(|text| {
        let text = text.trim();
        if text.is_empty() || text.len() > 8192 {
            Err(format!("{provider} returned empty or oversized OCR text"))
        } else {
            Ok(Recognition {
                text: text.into(),
                provider,
            })
        }
    });
    observation.finish(if result.is_ok() { "ok" } else { "error" });
    result
}

/// Errors are not winners. Returning drops the other future and its HTTP body;
/// remote servers may still finish work or bill after the client disconnects.
pub(super) async fn first_valid(
    local: impl Future<Output = Result<Recognition, String>>,
    cloud: impl Future<Output = Result<Recognition, String>>,
) -> Result<Recognition, String> {
    tokio::pin!(local, cloud);
    tokio::select! {
        result = &mut local => match result {
            Ok(value) => Ok(value),
            Err(local_error) => cloud.await.map_err(|cloud_error| format!("{local_error}; {cloud_error}")),
        },
        result = &mut cloud => match result {
            Ok(value) => Ok(value),
            Err(cloud_error) => local.await.map_err(|local_error| format!("{local_error}; {cloud_error}")),
        },
    }
}

/// Deliberately uncached and uncancelled: only used by the explicit benchmark.
pub struct ProviderBenchmark {
    pub provider: &'static str,
    pub elapsed: Duration,
    pub result: Result<String, String>,
}

pub(super) async fn benchmark(
    provider: &'static str,
    future: impl Future<Output = Result<String, String>>,
) -> ProviderBenchmark {
    let started = Instant::now();
    let result = observe(provider, future).await.map(|value| value.text);
    ProviderBenchmark {
        provider,
        elapsed: started.elapsed(),
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    async fn delayed(value: Result<&str, &str>) -> Result<String, String> {
        tokio::task::yield_now().await;
        value.map(str::to_string).map_err(str::to_string)
    }

    #[tokio::test]
    async fn fast_errors_and_empty_results_do_not_win() {
        for bad in [Err("unavailable"), Ok("   ")] {
            let winner = first_valid(
                observe("local", async {
                    bad.map(str::to_string).map_err(str::to_string)
                }),
                observe("openai", delayed(Ok("REMOVE IT"))),
            )
            .await
            .unwrap();
            assert_eq!(winner.provider, "openai");
            assert_eq!(winner.text, "REMOVE IT");
            let winner = first_valid(
                observe("local", delayed(Ok("REMOVE IT"))),
                observe("openai", async {
                    bad.map(str::to_string).map_err(str::to_string)
                }),
            )
            .await
            .unwrap();
            assert_eq!(winner.provider, "local");
        }
    }

    #[tokio::test]
    async fn both_failures_are_reported() {
        let result = first_valid(
            observe("local", delayed(Err("local failed"))),
            observe("openai", delayed(Err("cloud failed"))),
        )
        .await;
        let error = result.err().unwrap();
        assert!(error.contains("local failed") && error.contains("cloud failed"));
    }

    #[tokio::test]
    async fn oversized_output_is_not_a_valid_winner() {
        let result = first_valid(
            observe("local", delayed(Ok("text"))),
            observe("openai", async { Ok("x".repeat(8193)) }),
        )
        .await
        .unwrap();
        assert_eq!(result.provider, "local");
    }

    #[tokio::test]
    async fn winner_drops_the_pending_loser() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        for local_wins in [true, false] {
            let cancelled = Arc::new(AtomicBool::new(false));
            let guard = Dropped(cancelled.clone());
            let loser = async move {
                let _guard = guard;
                std::future::pending::<Result<Recognition, String>>().await
            };
            let winner = observe(
                if local_wins { "local" } else { "openai" },
                delayed(Ok("text")),
            );
            let result = if local_wins {
                first_valid(winner, loser).await
            } else {
                first_valid(loser, winner).await
            };
            assert_eq!(result.unwrap().text, "text");
            assert!(cancelled.load(Ordering::SeqCst));
        }
    }
}
