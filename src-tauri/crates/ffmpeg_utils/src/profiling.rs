//! Opt-in, per-job wall-clock attribution for the native graph worker.
//! CPU spans include blocking inside APIs; they are not CPU utilization or GPU
//! execution times. Nested spans have exclusive totals to prevent double counts.
//! Observations (including hardware timestamps) are separate and never added to
//! that partition. No timers, allocations or locks run when profiling is off.

use serde::Serialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct MediaProfiler(Arc<Mutex<ProfileReport>>);

#[derive(Clone, Default, Serialize)]
pub struct ProfileReport {
    pub metadata: BTreeMap<String, String>,
    pub spans: BTreeMap<String, Timing>,
    pub observations: BTreeMap<String, Timing>,
}

#[derive(Clone, Default, Serialize)]
pub struct Timing {
    pub calls: u64,
    pub inclusive_ms: f64,
    pub exclusive_ms: f64,
    pub max_ms: f64,
    /// Upper bounds from a fixed-size histogram (eight bins per power of two).
    pub p50_upper_ms: f64,
    pub p95_upper_ms: f64,
}

#[derive(Clone)]
struct Aggregate {
    calls: u64,
    total: Duration,
    exclusive: Duration,
    max: Duration,
    bins: [u64; 512],
}
impl Default for Aggregate {
    fn default() -> Self {
        Self {
            calls: 0,
            total: Duration::ZERO,
            exclusive: Duration::ZERO,
            max: Duration::ZERO,
            bins: [0; 512],
        }
    }
}
impl Aggregate {
    fn add(&mut self, elapsed: Duration, exclusive: Duration) {
        self.calls += 1;
        self.total += elapsed;
        self.exclusive += exclusive;
        self.max = self.max.max(elapsed);
        let bucket = ((elapsed.as_nanos().max(1) as f64).log2() * 8.0).floor() as usize;
        self.bins[bucket.min(511)] += 1;
    }
    fn timing(&self) -> Timing {
        let quantile = |fraction: f64| {
            let target = (self.calls as f64 * fraction).ceil() as u64;
            let mut count = 0;
            for (index, n) in self.bins.iter().enumerate() {
                count += n;
                if count >= target {
                    return 2f64.powf((index + 1) as f64 / 8.0) / 1e6;
                }
            }
            0.0
        };
        Timing {
            calls: self.calls,
            inclusive_ms: self.total.as_secs_f64() * 1000.0,
            exclusive_ms: self.exclusive.as_secs_f64() * 1000.0,
            max_ms: self.max.as_secs_f64() * 1000.0,
            p50_upper_ms: quantile(0.5),
            p95_upper_ms: quantile(0.95),
        }
    }
}

#[derive(Default)]
struct State {
    metadata: BTreeMap<String, String>,
    spans: BTreeMap<&'static str, Aggregate>,
    observations: BTreeMap<&'static str, Aggregate>,
    children: Vec<Duration>,
}
thread_local! { static ACTIVE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) }; }

impl MediaProfiler {
    /// Read after the worker completes. Concurrent jobs need separate collectors.
    pub fn report(&self) -> ProfileReport {
        self.0.lock().unwrap().clone()
    }
    pub(crate) fn start(&self) -> Session {
        let state = Rc::new(RefCell::new(State::default()));
        let previous = ACTIVE.with(|active| active.replace(Some(state.clone())));
        Session {
            state,
            previous,
            output: self.clone(),
        }
    }
}
pub(crate) struct Session {
    state: Rc<RefCell<State>>,
    previous: Option<Rc<RefCell<State>>>,
    output: MediaProfiler,
}
impl Drop for Session {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.replace(self.previous.take()));
        let state = self.state.borrow();
        debug_assert!(state.children.is_empty());
        let convert = |map: &BTreeMap<&str, Aggregate>| {
            map.iter()
                .map(|(key, value)| ((*key).to_owned(), value.timing()))
                .collect()
        };
        *self.output.0.lock().unwrap() = ProfileReport {
            metadata: state.metadata.clone(),
            spans: convert(&state.spans),
            observations: convert(&state.observations),
        };
    }
}

pub fn enabled() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

/// Record diagnostic configuration without evaluating it on the disabled path.
pub fn annotate(name: &str, value: impl FnOnce() -> String) {
    ACTIVE.with(|active| {
        if let Some(state) = active.borrow().as_ref() {
            state.borrow_mut().metadata.insert(name.to_owned(), value());
        }
    });
}

/// Synchronous worker span. Keep guards lexically nested on the same thread.
#[must_use]
pub struct Span(Option<(Rc<RefCell<State>>, &'static str, Instant)>);
pub fn span(name: &'static str) -> Span {
    Span(ACTIVE.with(|active| {
        active.borrow().as_ref().map(|state| {
            state.borrow_mut().children.push(Duration::ZERO);
            (state.clone(), name, Instant::now())
        })
    }))
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((state, name, start)) = &self.0 {
            let elapsed = start.elapsed();
            let mut state = state.borrow_mut();
            let children = state.children.pop().expect("Profile span nesting");
            state
                .spans
                .entry(name)
                .or_default()
                .add(elapsed, elapsed.saturating_sub(children));
            if let Some(parent) = state.children.last_mut() {
                *parent += elapsed;
            }
        }
    }
}
pub fn measure<T>(name: &'static str, work: impl FnOnce() -> T) -> T {
    let _span = span(name);
    work()
}
/// Supplemental elapsed interval, possibly overlapping spans or GPU work.
pub fn observe(name: &'static str, elapsed: Duration) {
    ACTIVE.with(|active| {
        if let Some(state) = active.borrow().as_ref() {
            state
                .borrow_mut()
                .observations
                .entry(name)
                .or_default()
                .add(elapsed, Duration::ZERO);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_spans_partition_wall_time_and_sessions_are_isolated() {
        let outer = MediaProfiler::default();
        let inner = MediaProfiler::default();
        {
            let _session = outer.start();
            measure("worker", || {
                measure("decode", || {
                    measure("child", || observe("gpu", Duration::from_millis(2)))
                });
                {
                    let _inner = inner.start();
                    measure("other_job", || ());
                }
            });
        }
        let report = outer.report();
        let sum: f64 = report.spans.values().map(|s| s.exclusive_ms).sum();
        assert!((sum - report.spans["worker"].inclusive_ms).abs() < 1e-6);
        assert!(!report.spans.contains_key("other_job"));
        assert_eq!(inner.report().spans.len(), 1);
        assert_eq!(report.observations["gpu"].inclusive_ms, 2.0);
        assert!(!enabled());
        measure("disabled", || observe("disabled", Duration::from_secs(1)));
        assert_eq!(outer.report().spans.len(), 3);
    }
    #[test]
    fn error_and_unwind_restore_worker_state() {
        let collector = MediaProfiler::default();
        let _ = std::panic::catch_unwind(|| {
            let _session = collector.start();
            measure("failed", || panic!("test"));
        });
        assert!(!enabled());
        assert_eq!(collector.report().spans["failed"].calls, 1);
    }
}
