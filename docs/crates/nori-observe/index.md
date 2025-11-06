---
layout: default
title: nori-observe
parent: Crates
nav_order: 1
---

# nori-observe

Vendor-neutral observability ABI for NoriKV and other Rust systems.

## Overview

`nori-observe` provides a minimal, composable abstraction for metrics and live visualization events. It allows crates to emit telemetry without depending on specific backends like Prometheus or OpenTelemetry.

## Features

- **Tiny traits**: `Meter`, `Counter`, `Gauge`, `Histogram` with minimal overhead
- **Typed events**: `VizEvent` enums for live dashboard streaming
- **Simple macros**: `obs_count!`, `obs_gauge!`, `obs_hist!`, `obs_timed!`
- **Zero-allocation budgets**: Counter ≤80ns, Histogram ≤200ns
- **NoopMeter** for testing and development

## Usage

```rust
use nori_observe::{Meter, obs_count, obs_timed};

// Inject meter via constructor
pub struct MyService {
    meter: Arc<dyn Meter>,
}

impl MyService {
    pub fn handle_request(&self) {
        // Increment counter
        obs_count!(self.meter, "requests_total", &[("method", "GET")]);

        // Time an operation
        obs_timed!(self.meter, "request_duration_ms", {
            // Your code here
        });
    }
}
```

## Backends

Implementations of the `Meter` trait:

- **nori-observe-prom** - Prometheus/OpenMetrics exporter
- **nori-observe-otlp** - OTLP gRPC exporter with trace exemplars
- **Custom** - Implement `Meter` for any backend

## Design Principles

1. **Vendor-neutral** - Core crates never import backend-specific dependencies
2. **Zero-allocation** - Hot paths use pre-allocated metric instances
3. **Composable** - Meters can wrap/decorate other meters
4. **Type-safe** - VizEvents are strongly typed enums

## Installation

```toml
[dependencies]
nori-observe = "0.1"
```

## Status

Production-ready. Used throughout NoriKV core crates.

## License

MIT OR Apache-2.0
