//! Flat JSON Lines encoding and typed span field collection.

use std::fmt;

use serde::Serializer;
use serde::ser::SerializeMap;
use serde_json::{Number, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::{Environment, LogConfig, ProcessName};

const STANDARD: [&str; 7] = ["ts", "level", "env", "server", "instance", "target", "msg"];

#[derive(Default)]
struct Fields(Vec<(&'static str, Value)>);

impl Fields {
    fn insert(&mut self, name: &'static str, value: Value) {
        self.take(name);
        self.0.push((name, value));
    }

    fn take(&mut self, name: &str) -> Option<Value> {
        let index = self.0.iter().position(|(key, _)| *key == name)?;
        Some(self.0.remove(index).1)
    }
}

impl Visit for Fields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field.name(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field.name(), value.into());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        if let Some(number) = Number::from_i128(value) {
            self.insert(field.name(), Value::Number(number));
        }
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        if let Some(number) = Number::from_u128(value) {
            self.insert(field.name(), Value::Number(number));
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let json =
            Number::from_f64(value).map_or_else(|| Value::String(value.to_string()), Value::Number);
        self.insert(field.name(), json);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field.name(), value.into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field.name(), value.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field.name(), format!("{value:?}").into());
    }
}

pub(super) struct SpanFields;

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanFields {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut fields = Fields::default();
            attrs.record(&mut fields);
            span.extensions_mut().insert(fields);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(fields) = span.extensions_mut().get_mut::<Fields>()
        {
            values.record(fields);
        }
    }
}

// The fmt layer only writes events; this layer already stores span fields as typed values.
impl FormatFields<'_> for SpanFields {
    fn format_fields<R: RecordFields>(&self, _: Writer<'_>, _: R) -> fmt::Result {
        Ok(())
    }
}

pub(super) struct Line {
    env: Environment,
    server: ProcessName,
    instance: String,
}

impl From<&LogConfig> for Line {
    fn from(config: &LogConfig) -> Self {
        Self {
            env: config.environment,
            server: config.name,
            instance: config.instance.0.clone(),
        }
    }
}

impl<S, N> FormatEvent<S, N> for Line
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut ts = String::new();
        SystemTime.format_time(&mut Writer::new(&mut ts))?;
        let mut fields = Fields::default();
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                if let Some(span_fields) = span.extensions().get::<Fields>() {
                    for (name, value) in &span_fields.0 {
                        fields.insert(name, value.clone());
                    }
                }
            }
        }
        let mut own_fields = Fields::default();
        event.record(&mut own_fields);
        let msg = match own_fields.take("message") {
            Some(Value::String(message)) => message,
            Some(value) => value.to_string(),
            None => String::new(),
        };
        for (name, value) in own_fields.0 {
            fields.insert(name, value);
        }
        let meta = event.metadata();
        let mut line = serde_json::Serializer::new(Vec::new());
        let mut emit = || -> serde_json::Result<()> {
            let mut map = line.serialize_map(None)?;
            map.serialize_entry("ts", &ts)?;
            map.serialize_entry("level", meta.level().as_str())?;
            map.serialize_entry("env", &self.env)?;
            map.serialize_entry("server", &self.server)?;
            map.serialize_entry("instance", &self.instance)?;
            map.serialize_entry("target", meta.target())?;
            map.serialize_entry("msg", &msg)?;
            for (name, value) in &fields.0 {
                let base = name.trim_end_matches('_');
                // A literal suffixed key moves only if the preceding escaped key needs its slot.
                if STANDARD.contains(&base)
                    && (base.len()..name.len()).all(|len| {
                        name.get(..len)
                            .is_some_and(|prefix| fields.0.iter().any(|(key, _)| *key == prefix))
                    })
                {
                    map.serialize_entry(&format!("{name}_"), value)?;
                } else {
                    map.serialize_entry(name, value)?;
                }
            }
            map.end()
        };
        emit().map_err(|_| fmt::Error)?;
        let bytes = line.into_inner();
        // The fmt layer buffers this complete event and makes one write to the appender.
        writeln!(
            writer,
            "{}",
            std::str::from_utf8(&bytes).map_err(|_| fmt::Error)?
        )
    }
}
