//! Prometheus sample fields used by the monitoring pages.

pub(super) const RUNTIME_METRICS: [&str; 3] = [
    "runtime_workers",
    "runtime_alive_tasks",
    "runtime_global_queue_depth",
];

#[derive(Debug)]
pub(super) struct Sample<'a> {
    pub(super) name: &'a str,
    pub(super) labels: Option<&'a str>,
    pub(super) value: &'a str,
}

impl<'a> Sample<'a> {
    pub(super) fn label(&self, key: &str) -> Option<&'a str> {
        self.labels?.split(',').find_map(|field| {
            let (name, value) = field.split_once('=')?;
            (name == key)
                .then_some(value)?
                .strip_prefix('"')?
                .strip_suffix('"')
        })
    }
}

pub(super) fn samples(text: &str) -> impl Iterator<Item = Sample<'_>> {
    text.lines().filter_map(|line| {
        let line = line.trim_start();
        let boundary = line.find(|ch: char| ch == '{' || ch.is_ascii_whitespace())?;
        let (name, rest) = line.split_at(boundary);
        let (labels, values) = if let Some(labels) = rest.strip_prefix('{') {
            let (labels, values) = labels.rsplit_once('}')?;
            (Some(labels), values)
        } else {
            (None, rest)
        };
        let value = values.split_ascii_whitespace().next()?;
        (name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
            && value.parse::<f64>().is_ok())
        .then_some(Sample {
            name,
            labels,
            value,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::samples;

    #[test]
    fn metrics_keep_numeric_samples_and_labels() {
        for (text, expected) in [
            (
                "# TYPE server_ready gauge\nconnection_count 2\nserver_ready 1\n",
                vec![("connection_count", None, "2"), ("server_ready", None, "1")],
            ),
            (
                "bad nope\nwith_labels{shard=\"first shard\"} 5\nduration 1.2e-2 123456\n",
                vec![
                    ("with_labels", Some("shard=\"first shard\""), "5"),
                    ("duration", None, "1.2e-2"),
                ],
            ),
            ("\n# help\nmissing\n", vec![]),
        ] {
            assert_eq!(
                samples(text)
                    .map(|sample| (sample.name, sample.labels, sample.value))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
