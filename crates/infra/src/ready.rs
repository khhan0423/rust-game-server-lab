//! The line a server writes to announce its bound ports to its parent.

use std::{fmt, io, net::SocketAddr, str::FromStr};

/// Bound listener addresses in announcement order, including the admin port.
#[derive(Debug)]
pub struct ReadyLine(Vec<(String, SocketAddr)>);

impl ReadyLine {
    /// Builds the announcement, rejecting ambiguous names before it is written.
    pub fn new(admin: SocketAddr, listeners: &[(&str, SocketAddr)]) -> io::Result<Self> {
        let mut line = Self(vec![("admin".to_owned(), admin)]);
        for &(name, address) in listeners {
            line.insert(name, address)?;
        }
        Ok(line)
    }

    fn insert(&mut self, name: &str, address: SocketAddr) -> io::Result<()> {
        if name.is_empty()
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || self.addr(name).is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid or duplicate listener name",
            ));
        }
        self.0.push((name.to_owned(), address));
        Ok(())
    }

    /// Looks up a listener by name, independent of field order.
    pub fn addr(&self, name: &str) -> Option<SocketAddr> {
        self.0
            .iter()
            .find_map(|(key, addr)| (key == name).then_some(*addr))
    }
}

impl fmt::Display for ReadyLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ready")?;
        for (name, addr) in &self.0 {
            write!(f, " {name}={addr}")?;
        }
        Ok(())
    }
}

impl FromStr for ReadyLine {
    type Err = io::Error;

    fn from_str(text: &str) -> io::Result<Self> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid ready announcement");
        let mut fields = text.split_ascii_whitespace();
        if fields.next() != Some("ready") {
            return Err(invalid());
        }
        let mut line = Self(Vec::new());
        for field in fields {
            let (key, addr) = field.split_once('=').ok_or_else(invalid)?;
            line.insert(key, addr.parse().map_err(|_| invalid())?)?;
        }
        line.addr("admin").ok_or_else(invalid)?;
        Ok(line)
    }
}
