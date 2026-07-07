use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

const LOG_INTERVAL: Duration = Duration::from_secs(5);
const MIN_SAMPLES: usize = 120;

static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("HELIX_PROFILE_FRAMES").is_some_and(|value| value == "1"));

static PROFILER: LazyLock<Profiler> = LazyLock::new(Profiler::default);

pub fn enabled() -> bool {
    *ENABLED
}

pub fn scope(name: &'static str) -> Scope {
    if !enabled() {
        return Scope::disabled();
    }

    Scope {
        start: Some(Instant::now()),
        metric: Some(PROFILER.metric(name)),
    }
}

#[derive(Default)]
struct Profiler {
    metrics: Mutex<HashMap<&'static str, Arc<Metric>>>,
}

impl Profiler {
    fn metric(&self, name: &'static str) -> Arc<Metric> {
        let mut metrics = self.metrics.lock().unwrap();
        metrics
            .entry(name)
            .or_insert_with(|| Arc::new(Metric::new(name)))
            .clone()
    }
}

struct Metric {
    name: &'static str,
    samples: Mutex<Samples>,
    logging: AtomicBool,
}

impl Metric {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            samples: Mutex::new(Samples {
                values: Vec::with_capacity(MIN_SAMPLES),
                last_log: Instant::now(),
            }),
            logging: AtomicBool::new(false),
        }
    }

    fn record(&self, elapsed: Duration) {
        let mut samples = self.samples.lock().unwrap();
        samples.values.push(elapsed.as_micros() as u64);

        if samples.values.len() < MIN_SAMPLES || samples.last_log.elapsed() < LOG_INTERVAL {
            return;
        }

        if self.logging.swap(true, Ordering::Relaxed) {
            return;
        }

        let values = std::mem::take(&mut samples.values);
        samples.last_log = Instant::now();
        drop(samples);

        let stats = Stats::from_values(values);
        log::info!(
            "frame profile {}: samples={} p50={:.2}ms p95={:.2}ms max={:.2}ms",
            self.name,
            stats.samples,
            stats.p50_ms(),
            stats.p95_ms(),
            stats.max_ms(),
        );
        self.logging.store(false, Ordering::Relaxed);
    }
}

struct Samples {
    values: Vec<u64>,
    last_log: Instant,
}

struct Stats {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
    max_us: u64,
}

impl Stats {
    fn from_values(mut values: Vec<u64>) -> Self {
        debug_assert!(!values.is_empty());
        values.sort_unstable();

        let samples = values.len();
        let percentile = |percent: usize| {
            let index = samples.saturating_sub(1) * percent / 100;
            values[index]
        };

        Self {
            samples,
            p50_us: percentile(50),
            p95_us: percentile(95),
            max_us: *values.last().unwrap(),
        }
    }

    fn p50_ms(&self) -> f64 {
        micros_to_millis(self.p50_us)
    }

    fn p95_ms(&self) -> f64 {
        micros_to_millis(self.p95_us)
    }

    fn max_ms(&self) -> f64 {
        micros_to_millis(self.max_us)
    }
}

fn micros_to_millis(micros: u64) -> f64 {
    micros as f64 / 1_000.0
}

pub struct Scope {
    start: Option<Instant>,
    metric: Option<Arc<Metric>>,
}

impl Scope {
    fn disabled() -> Self {
        Self {
            start: None,
            metric: None,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let (Some(start), Some(metric)) = (self.start, &self.metric) {
            metric.record(start.elapsed());
        }
    }
}
