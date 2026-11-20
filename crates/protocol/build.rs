//! Compile the pilot schemas and derive the dispatch registry from the catalog.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;

const CATALOG: &str = "../../docs/protocol/message-catalog.md";
const CATALOG_KO: &str = "../../docs/protocol/message-catalog.ko.md";
const SCHEMAS: [&str; 5] = [
    "proto/common.proto",
    "proto/system.proto",
    "proto/content.proto",
    "proto/zone.proto",
    "proto/control.proto",
];

#[derive(Debug, PartialEq, Eq)]
struct Row<'a> {
    id: &'a str,
    name: &'a str,
    direction: &'a str,
    delivery: &'a str,
    kind: &'static str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = io::stdout().lock();
    for path in SCHEMAS.into_iter().chain([CATALOG, CATALOG_KO]) {
        writeln!(cargo, "cargo::rerun-if-changed={path}")?;
    }
    writeln!(cargo, "cargo::rerun-if-env-changed=PROTOC")?;
    drop(cargo);
    prost_build::compile_protos(&SCHEMAS, &["proto"])?;
    let catalog = fs::read_to_string(CATALOG)?;
    let korean = fs::read_to_string(CATALOG_KO)?;
    let specs = rows(&catalog)?;
    if specs != rows(&korean)? {
        return Err(io::Error::other("English and Korean message registries differ").into());
    }
    let mut registry = String::new();
    for (packet, id, predicate) in [
        ("ClientPacket", "MessageId", client as fn(&Row<'_>) -> bool),
        ("ServerPacket", "MessageId", server),
        ("ControlPacket", "ControlId", control),
    ] {
        writeln!(registry, "packets!({packet}, {id};")?;
        for row in specs.iter().filter(|row| predicate(row)) {
            writeln!(registry, "    {} = {},", row.name, row.id)?;
        }
        writeln!(registry, ");")?;
    }
    writeln!(registry, "message_ids!(")?;
    for row in specs.iter().filter(|row| !control(row)) {
        let kind = row.kind;
        let direction = match row.direction {
            "C\u{2192}S" => "ClientToServer",
            "S\u{2192}C" => "ServerToClient",
            "C\u{2194}S" => "Both",
            other => return Err(io::Error::other(format!("invalid direction {other}")).into()),
        };
        writeln!(
            registry,
            "    {} = {}, {kind}, {direction}, {};",
            constant(row.name),
            row.id,
            row.delivery
        )?;
    }
    writeln!(registry, ");")?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("registry.rs"), registry)?;
    Ok(())
}

fn rows(catalog: &str) -> io::Result<Vec<Row<'_>>> {
    let mut rows = Vec::new();
    for line in catalog.lines() {
        if !line.starts_with("| `0x") {
            continue;
        }
        let mut fields = line.split('|').map(str::trim);
        let row = match (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) {
            (Some(""), Some(id), Some(name), Some(direction), Some(delivery)) => {
                let id = id.trim_matches('`');
                Row {
                    id,
                    name: name.trim_matches('`'),
                    direction,
                    delivery,
                    kind: kind(id, delivery)?,
                }
            }
            _ => return Err(io::Error::other(format!("invalid catalog row: {line}"))),
        };
        rows.push(row);
    }
    Ok(rows)
}

/// Classifies a row by its ID range (design 02 section 3.2), so no heading text is parsed.
fn kind(id: &str, delivery: &str) -> io::Result<&'static str> {
    if delivery == "N/A" {
        return Ok("Control");
    }
    let value = u16::from_str_radix(id.trim_start_matches("0x"), 16)
        .map_err(|_| io::Error::other(format!("invalid message id {id}")))?;
    Ok(match value {
        0x0000..=0x0FFF => "System",
        0x1000..=0x7FFF => "Content",
        0x8000..=u16::MAX => "Zone",
    })
}

fn client(row: &Row<'_>) -> bool {
    matches!(row.direction, "C\u{2192}S" | "C\u{2194}S")
}

fn server(row: &Row<'_>) -> bool {
    matches!(row.direction, "S\u{2192}C" | "C\u{2194}S")
}

fn control(row: &Row<'_>) -> bool {
    row.delivery == "N/A"
}

fn constant(name: &str) -> String {
    let mut result = String::new();
    for (index, ch) in name.chars().enumerate() {
        if index != 0 && ch.is_ascii_uppercase() {
            result.push('_');
        }
        result.push(ch.to_ascii_uppercase());
    }
    result
}
