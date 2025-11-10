//! Prometheus metrics implementation.
//!
//! Implements nori-observe::Meter trait using prometheus-client.

use nori_observe::{Counter, Gauge, Histogram, Meter, VizEvent};
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter as PromCounter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge as PromGauge;
use prometheus_client::metrics::histogram::Histogram as PromHistogram;
use prometheus_client::registry::Registry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Prometheus metrics collector.
///
/// Implements the nori-observe::Meter trait using prometheus-client.
pub struct PrometheusMeter {
    registry: Arc<Mutex<Registry>>,
    counters: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromCounter>>>>,
    gauges: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromGauge>>>>,
    histograms: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromHistogram>>>>,
}

impl PrometheusMeter {
    /// Create a new Prometheus meter with a fresh registry.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(Registry::default())),
            counters: Arc::new(Mutex::new(HashMap::new())),
            gauges: Arc::new(Mutex::new(HashMap::new())),
            histograms: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Export metrics in Prometheus text format.
    pub fn export(&self) -> String {
        let registry = self.registry.lock().unwrap();
        let mut buffer = String::new();
        encode(&mut buffer, &registry).unwrap();
        buffer
    }

    /// Get or create a counter family.
    fn get_or_create_counter(&self, name: &str) -> Family<Vec<(String, String)>, PromCounter> {
        let mut counters = self.counters.lock().unwrap();

        if let Some(family) = counters.get(name) {
            return family.clone();
        }

        // Create new counter family
        let family = Family::<Vec<(String, String)>, PromCounter>::default();

        // Register with registry
        let mut registry = self.registry.lock().unwrap();
        registry.register(name, "Counter", family.clone());

        counters.insert(name.to_string(), family.clone());
        family
    }

    /// Get or create a gauge family.
    fn get_or_create_gauge(&self, name: &str) -> Family<Vec<(String, String)>, PromGauge> {
        let mut gauges = self.gauges.lock().unwrap();

        if let Some(family) = gauges.get(name) {
            return family.clone();
        }

        // Create new gauge family
        let family = Family::<Vec<(String, String)>, PromGauge>::default();

        // Register with registry
        let mut registry = self.registry.lock().unwrap();
        registry.register(name, "Gauge", family.clone());

        gauges.insert(name.to_string(), family.clone());
        family
    }

    /// Get or create a histogram family.
    fn get_or_create_histogram(&self, name: &str) -> Family<Vec<(String, String)>, PromHistogram> {
        let mut histograms = self.histograms.lock().unwrap();

        if let Some(family) = histograms.get(name) {
            return family.clone();
        }

        // Create new histogram family with default buckets
        let family = Family::<Vec<(String, String)>, PromHistogram>::new_with_constructor(|| {
            PromHistogram::new(prometheus_client::metrics::histogram::exponential_buckets(
                1.0, 2.0, 10,
            ))
        });

        // Register with registry
        let mut registry = self.registry.lock().unwrap();
        registry.register(name, "Histogram", family.clone());

        histograms.insert(name.to_string(), family.clone());
        family
    }
}

impl Default for PrometheusMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper for prometheus-client counter.
struct PrometheusCounter {
    counter: PromCounter,
}

impl Counter for PrometheusCounter {
    fn inc(&self, v: u64) {
        self.counter.inc_by(v);
    }
}

/// Wrapper for prometheus-client gauge.
struct PrometheusGauge {
    gauge: PromGauge,
}

impl Gauge for PrometheusGauge {
    fn set(&self, v: i64) {
        self.gauge.set(v);
    }
}

/// Wrapper for prometheus-client histogram.
struct PrometheusHistogram {
    histogram: PromHistogram,
}

impl Histogram for PrometheusHistogram {
    fn observe(&self, v: f64) {
        self.histogram.observe(v);
    }
}

impl Meter for PrometheusMeter {
    fn counter(
        &self,
        name: &'static str,
        labels: &'static [(&'static str, &'static str)],
    ) -> Box<dyn Counter> {
        let family = self.get_or_create_counter(name);

        // Convert labels to Vec<(String, String)>
        let label_vec: Vec<(String, String)> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let counter = family.get_or_create(&label_vec).clone();

        Box::new(PrometheusCounter { counter })
    }

    fn gauge(
        &self,
        name: &'static str,
        labels: &'static [(&'static str, &'static str)],
    ) -> Box<dyn Gauge> {
        let family = self.get_or_create_gauge(name);

        // Convert labels to Vec<(String, String)>
        let label_vec: Vec<(String, String)> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let gauge = family.get_or_create(&label_vec).clone();

        Box::new(PrometheusGauge { gauge })
    }

    fn histo(
        &self,
        name: &'static str,
        _buckets: &'static [f64],
        labels: &'static [(&'static str, &'static str)],
    ) -> Box<dyn Histogram> {
        let family = self.get_or_create_histogram(name);

        // Convert labels to Vec<(String, String)>
        let label_vec: Vec<(String, String)> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let histogram = family.get_or_create(&label_vec).clone();

        Box::new(PrometheusHistogram { histogram })
    }

    fn emit(&self, _evt: VizEvent) {
        // VizEvents are for live visualization, not Prometheus
        // Could be forwarded to a separate event stream if needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter() {
        let meter = PrometheusMeter::new();
        let counter = meter.counter("test_counter", &[("label", "value")]);
        counter.inc(5);
        counter.inc(3);

        let output = meter.export();
        assert!(output.contains("test_counter"));
    }

    #[test]
    fn test_gauge() {
        let meter = PrometheusMeter::new();
        let gauge = meter.gauge("test_gauge", &[("label", "value")]);
        gauge.set(42);

        let output = meter.export();
        assert!(output.contains("test_gauge"));
    }

    #[test]
    fn test_histogram() {
        let meter = PrometheusMeter::new();
        let histo = meter.histo("test_histogram", &[], &[("label", "value")]);
        histo.observe(1.5);
        histo.observe(2.5);
        histo.observe(3.5);

        let output = meter.export();
        assert!(output.contains("test_histogram"));
    }
}
