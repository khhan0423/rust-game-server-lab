use std::collections::BTreeMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use serde::Serialize;

use crate::RunConfig;
use crate::session::{End, Outcome};

/// Latency distribution in milliseconds; the histogram records microseconds.
#[derive(Debug, Serialize)]
pub struct Latency {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Final counts and latency measurements for one run.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub attempted: u32,
    pub established: u32,
    pub failed_to_connect: u32,
    pub closed_by_server: BTreeMap<&'static str, u32>,
    pub closed_by_timeout: u32,
    pub closed_by_io_error: u32,
    pub cancelled: u32,
    pub interrupted: bool,
    pub connect_latency_ms: Option<Latency>,
    pub auth_to_close_ms: Option<Latency>,
    pub wall_time_ms: u128,
}

impl Summary {
    /// Every established socket must receive the requested reason when one is specified.
    pub fn meets_expectations(&self, config: &RunConfig) -> bool {
        !self.interrupted
            && config
                .expect_established
                .is_none_or(|expected| self.established == expected)
            && config.expect_close_reason.as_deref().is_none_or(|reason| {
                self.established > 0 && self.closed_by_server.get(reason) == Some(&self.established)
            })
    }
}

pub(super) struct Collector {
    pub(super) attempted: u32,
    pub(super) failed_to_connect: u32,
    server: BTreeMap<&'static str, u32>,
    timeout: u32,
    io: u32,
    cancelled: u32,
    established: u32,
    connect: Histogram<u64>,
    auth: Histogram<u64>,
}

impl Collector {
    pub(super) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            attempted: 0,
            failed_to_connect: 0,
            server: BTreeMap::new(),
            timeout: 0,
            io: 0,
            cancelled: 0,
            established: 0,
            connect: Histogram::new(3)?,
            auth: Histogram::new(3)?,
        })
    }

    pub(super) fn record(&mut self, outcome: Outcome) -> anyhow::Result<()> {
        match outcome {
            Outcome::FailedConnect => self.failed_to_connect += 1,
            Outcome::CancelledConnect => self.cancelled += 1,
            Outcome::Established {
                latency,
                end,
                auth_close,
            } => {
                self.established += 1;
                self.connect.record(u64::try_from(latency.as_micros())?)?;
                if let Some(latency) = auth_close {
                    self.auth.record(u64::try_from(latency.as_micros())?)?;
                }
                match end {
                    End::Server(reason) => *self.server.entry(reason).or_default() += 1,
                    End::Timeout => self.timeout += 1,
                    End::Io => self.io += 1,
                    End::Cancelled => self.cancelled += 1,
                }
            }
        }
        Ok(())
    }

    pub(super) const fn closed(&self) -> u32 {
        self.established
    }

    pub(super) fn finish(self, elapsed: Duration, interrupted: bool) -> Summary {
        Summary {
            attempted: self.attempted,
            established: self.established,
            failed_to_connect: self.failed_to_connect,
            closed_by_server: self.server,
            closed_by_timeout: self.timeout,
            closed_by_io_error: self.io,
            cancelled: self.cancelled,
            interrupted,
            connect_latency_ms: distribution(&self.connect),
            auth_to_close_ms: distribution(&self.auth),
            wall_time_ms: elapsed.as_millis(),
        }
    }
}

fn distribution(values: &Histogram<u64>) -> Option<Latency> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "Histogram samples above 2^53 microseconds cannot occur in a practical run"
    )]
    let millis = |value: u64| value as f64 / 1000.0;
    (!values.is_empty()).then(|| Latency {
        p50: millis(values.value_at_quantile(0.5)),
        p95: millis(values.value_at_quantile(0.95)),
        p99: millis(values.value_at_quantile(0.99)),
        max: millis(values.max()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scenario;

    #[test]
    fn distribution_has_no_empty_percentiles_and_converts_microseconds() {
        for (samples, expected) in [
            (&[][..], None),
            (&[250, 500, 750, 1000], Some([0.5, 1.0, 1.0, 1.0])),
        ] {
            let mut histogram = Histogram::<u64>::new(3).unwrap();
            for &sample in samples {
                histogram.record(sample).unwrap();
            }
            assert_eq!(
                distribution(&histogram).map(|value| [value.p50, value.p95, value.p99, value.max]),
                expected
            );
        }
    }

    #[test]
    fn expectations_require_matching_counts_and_every_established_close_reason() {
        for (established, matched, expected, interrupted, passes) in [
            (3, 3, Some(3), false, true),
            (3, 2, Some(3), false, false),
            (3, 3, Some(4), false, false),
            (0, 0, None, false, false),
            (3, 3, None, true, false),
        ] {
            let config = RunConfig {
                gateway: "127.0.0.1:18400".parse().unwrap(),
                connections: 3,
                ramp_per_sec: "100".parse().unwrap(),
                hold_secs: 1,
                expect_established: expected,
                expect_close_reason: Some("auth_failed".into()),
                scenario: Scenario::ConnectAuth,
            };
            let mut summary = Collector::new().unwrap();
            for index in 0..established {
                let end = if index < matched {
                    End::Server("auth_failed")
                } else {
                    End::Timeout
                };
                summary
                    .record(Outcome::Established {
                        latency: Duration::ZERO,
                        end,
                        auth_close: None,
                    })
                    .unwrap();
            }
            assert_eq!(
                summary
                    .finish(Duration::ZERO, interrupted)
                    .meets_expectations(&config),
                passes
            );
        }
    }
}
