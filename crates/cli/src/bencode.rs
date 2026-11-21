//! Minimal bencode reader: enough for `.torrent` files.
//!
//! The infohash is the SHA-1 of the *raw* encoded `info` dictionary, so the
//! parser keeps byte offsets instead of rebuilding the value.

use sha1::{Digest, Sha1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(Vec<(Vec<u8>, Value)>),
}

impl Value {
    fn get(&self, key: &[u8]) -> Option<&Value> {
        match self {
            Self::Dict(entries) => entries
                .iter()
                .find(|(name, _)| name.as_slice() == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

/// One torrent's catalogue-relevant fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metainfo {
    pub info_hash: String,
    pub name: String,
    pub total_bytes: u64,
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8, String> {
        let byte = *self
            .data
            .get(self.pos)
            .ok_or_else(|| "unexpected end of bencode".to_owned())?;
        self.pos += 1;
        Ok(byte)
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.byte()? {
            b'i' => {
                let start = self.pos;
                while self.byte()? != b'e' {}
                let text = std::str::from_utf8(&self.data[start..self.pos - 1])
                    .map_err(|error| error.to_string())?;
                text.parse()
                    .map(Value::Int)
                    .map_err(|_| format!("bad integer: {text}"))
            }
            b'l' => {
                let mut items = Vec::new();
                while *self
                    .data
                    .get(self.pos)
                    .ok_or_else(|| "unexpected end of list".to_owned())?
                    != b'e'
                {
                    items.push(self.value()?);
                }
                self.pos += 1;
                Ok(Value::List(items))
            }
            b'd' => {
                let mut entries = Vec::new();
                while *self
                    .data
                    .get(self.pos)
                    .ok_or_else(|| "unexpected end of dict".to_owned())?
                    != b'e'
                {
                    let key = self.text()?;
                    entries.push((key, self.value()?));
                }
                self.pos += 1;
                Ok(Value::Dict(entries))
            }
            digit if digit.is_ascii_digit() => {
                self.pos -= 1;
                Ok(Value::Bytes(self.text()?))
            }
            other => Err(format!(
                "unexpected bencode byte {other:#x} at {}",
                self.pos - 1
            )),
        }
    }

    fn text(&mut self) -> Result<Vec<u8>, String> {
        let start = self.pos;
        while self.data.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        let length: usize = std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|error| error.to_string())?
            .parse()
            .map_err(|_| "bad string length".to_owned())?;
        if self.byte()? != b':' {
            return Err("missing ':' after string length".to_owned());
        }
        let end = self.pos + length;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| "string runs past the buffer".to_owned())?;
        self.pos = end;
        Ok(bytes.to_vec())
    }
}

/// Parses a `.torrent` file down to the fields the catalogue and the importer
/// need. `info` must be the last-stored raw bytes for a correct infohash, so
/// the span is taken from the stream, not from a re-encoding.
pub fn parse(data: &[u8]) -> Result<Metainfo, String> {
    let mut parser = Parser::new(data);
    if parser.byte()? != b'd' {
        return Err("a torrent must start with a dictionary".to_owned());
    }
    let mut info: Option<(usize, usize)> = None;
    let mut root_entries = Vec::new();
    while *parser
        .data
        .get(parser.pos)
        .ok_or_else(|| "unexpected end of torrent".to_owned())?
        != b'e'
    {
        let key = parser.text()?;
        if key == b"info" {
            let start = parser.pos;
            let value = parser.value()?;
            info = Some((start, parser.pos));
            root_entries.push((key, value));
        } else {
            root_entries.push((key, parser.value()?));
        }
    }

    let (start, end) = info.ok_or_else(|| "torrent has no info dictionary".to_owned())?;
    let info_hash = hex(&Sha1::digest(&data[start..end]));
    let root = Value::Dict(root_entries);
    let info_value = root
        .get(b"info")
        .ok_or_else(|| "torrent has no info dictionary".to_owned())?;
    let name = info_value
        .get(b"name")
        .and_then(Value::as_bytes)
        .map_or_else(
            || info_hash.clone(),
            |name| String::from_utf8_lossy(name).into_owned(),
        );

    let total_bytes = match info_value.get(b"length").and_then(Value::as_int) {
        Some(length) => u64::try_from(length).unwrap_or(0),
        None => match info_value.get(b"files") {
            Some(Value::List(files)) => files
                .iter()
                .filter_map(|file| file.get(b"length").and_then(Value::as_int))
                .map(|length| u64::try_from(length).unwrap_or(0))
                .sum(),
            _ => 0,
        },
    };

    Ok(Metainfo {
        info_hash,
        name,
        total_bytes,
    })
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE: &[u8] =
        b"d8:announce29:http://tracker.local/announce4:infod6:lengthi1000e4:name9:alpha.binee";

    #[test]
    fn reads_single_file_torrents() {
        let metainfo = parse(SINGLE).expect("parse");
        assert_eq!(metainfo.name, "alpha.bin");
        assert_eq!(metainfo.total_bytes, 1000);
        assert_eq!(
            metainfo.info_hash,
            "153f3ff93ab819ced753ccef6bf481070c3a140c"
        );
    }

    #[test]
    fn sums_multi_file_torrents() {
        let data = b"d4:infod5:filesld6:lengthi10e4:name1:aed6:lengthi32e4:name1:bee4:name5:multi12:piece lengthi16384eee";
        let metainfo = parse(data).expect("parse");
        assert_eq!(metainfo.name, "multi");
        assert_eq!(metainfo.total_bytes, 42);
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(parse(b"d4:infod6:lengthi10e4:name1:a").is_err());
        assert!(parse(b"4:info").is_err());
    }

    #[test]
    fn infohash_uses_the_raw_info_bytes() {
        // Re-encoding would sort keys; the hash must come from the stream.
        let data = b"d4:infod4:name1:a6:lengthi1eee";
        let metainfo = parse(data).expect("parse");
        let digest = Sha1::digest(b"d4:name1:a6:lengthi1ee");
        assert_eq!(metainfo.info_hash, hex(&digest));
    }
}
