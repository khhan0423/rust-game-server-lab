//! Tests the operations log at its writer and file boundaries.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;

#[derive(Default)]
struct Lines(Mutex<Vec<Vec<u8>>>);

struct Sink(Arc<Lines>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = &'a Lines;

    fn make_writer(&'a self) -> Self::Writer {
        &self.0
    }
}

impl io::Write for &Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn event_lines_keep_ordered_identity_and_flatten_typed_fields() {
    let config: LogConfig = toml::from_str(
        "name='gateway'\ninstance='test'\nenvironment='friends_test'\nlog_level='info'",
    )
    .unwrap();
    let lines = Arc::new(Lines::default());
    let output = Arc::clone(&lines);

    tracing::subscriber::with_default(subscriber(&config, Sink(output)), || {
        tracing::info_span!("outer", conn_id = 69_u64, request_id = 1_u64, peer = "outer")
            .in_scope(|| {
                tracing::info_span!("inner", request_id = tracing::field::Empty, peer = "inner")
                    .in_scope(|| {
                        tracing::Span::current().record("request_id", 2_u64);
                        tracing::info!(target: "check", env = "span value\ncontinued", active = true, elapsed_ms = u128::MAX, "first");
                        tracing::info!(target: "check", peer = "event", "second");
                    });
            });
    });

    let writes = lines.0.lock().unwrap();
    let [first, second] = writes.as_slice() else {
        panic!("expected one write per event: {writes:?}");
    };
    for (bytes, msg, peer, count) in [
        (first, "first", "inner", 13),
        (second, "second", "event", 10),
    ] {
        let line = std::str::from_utf8(bytes).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(line.ends_with('\n'));
        let value: Value = serde_json::from_str(line).unwrap();
        let ts = value["ts"].as_str().unwrap();
        assert_eq!(ts.len(), 27);
        assert!(ts.bytes().enumerate().all(|(i, ch)| match i {
            4 | 7 => ch == b'-',
            10 => ch == b'T',
            13 | 16 => ch == b':',
            19 => ch == b'.',
            26 => ch == b'Z',
            _ => ch.is_ascii_digit(),
        }));
        assert!(line.starts_with(&format!(
            "{{\"ts\":\"{ts}\",\"level\":\"INFO\",\"env\":\"friends_test\",\"server\":\"gateway\",\"instance\":\"test\",\"target\":\"check\",\"msg\":\"{msg}\","
        )));
        assert_eq!(value.as_object().unwrap().len(), count);
        assert_eq!(value["conn_id"], 69);
        assert_eq!(value["request_id"], 2);
        assert_eq!(value["peer"], peer);
    }
    let first: Value = serde_json::from_slice(first).unwrap();
    assert_eq!(first["env_"], "span value\ncontinued");
    assert_eq!(first["active"], json!(true));
    assert_eq!(first["elapsed_ms"], json!(u128::MAX));
}

#[test]
fn daily_appender_derives_the_directory_and_filename() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp")
        .join(format!("infra-log-{}", std::process::id()));
    let mut config: LogConfig =
        toml::from_str("name='game'\nenvironment='development'\nlog_level='info'").unwrap();
    config.log_root.clone_from(&root);

    drop(appender(&config).unwrap());

    let dir = root.join("game-1");
    let files = std::fs::read_dir(dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let [file] = files.as_slice() else {
        panic!("expected exactly one daily log: {files:?}");
    };
    let filename = file.file_name();
    let date = filename
        .to_str()
        .unwrap()
        .strip_prefix("ops.")
        .unwrap()
        .strip_suffix(".jsonl")
        .unwrap();
    assert_eq!(date.len(), 10);
    assert!(date.bytes().enumerate().all(|(i, ch)| match i {
        4 | 7 => ch == b'-',
        _ => ch.is_ascii_digit(),
    }));
    std::fs::remove_dir_all(root).unwrap();
}
