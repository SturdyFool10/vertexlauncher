//! Minimal lossless NBT model, enough to rewrite `servers.dat` and `hotbar.nbt` without
//! disturbing fields this launcher doesn't understand.

use std::io::Read;

const MAX_DEPTH: usize = 512;

/// Ordered list of named tags; order is preserved so files round-trip unchanged.
pub type Compound = Vec<(String, Tag)>;

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    /// Raw bytes (Java modified UTF-8), kept verbatim so unusual characters survive.
    String(Vec<u8>),
    List(TagList),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagList {
    /// Element type id, kept even when the list is empty.
    pub element_id: u8,
    pub items: Vec<Tag>,
}

/// A whole NBT file: the root compound plus its (usually empty) name.
#[derive(Clone, Debug, PartialEq)]
pub struct NbtFile {
    pub name: String,
    pub root: Compound,
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(_) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    pub fn as_str(&self) -> Option<String> {
        match self {
            Tag::String(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Tag::Int(value) => Some(*value),
            _ => None,
        }
    }
}

pub fn get<'a>(compound: &'a Compound, key: &str) -> Option<&'a Tag> {
    compound
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, tag)| tag)
}

pub fn get_mut<'a>(compound: &'a mut Compound, key: &str) -> Option<&'a mut Tag> {
    compound
        .iter_mut()
        .find(|(name, _)| name == key)
        .map(|(_, tag)| tag)
}

/// Parses an NBT file, transparently handling gzip.
pub fn parse(bytes: &[u8]) -> Result<NbtFile, String> {
    let mut decompressed;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        decompressed = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut decompressed)
            .map_err(|err| format!("invalid gzip data: {err}"))?;
        decompressed.as_slice()
    } else {
        bytes
    };
    let mut reader = Reader { bytes, pos: 0 };
    let id = reader.u8()?;
    if id != 10 {
        return Err(format!("root tag is {id}, expected a compound"));
    }
    let name = String::from_utf8_lossy(&reader.string_bytes()?).into_owned();
    let root = reader.compound(0)?;
    Ok(NbtFile { name, root })
}

/// Serializes an NBT file, uncompressed (the format `servers.dat` and `hotbar.nbt` use).
pub fn write(file: &NbtFile) -> Vec<u8> {
    let mut out = vec![10];
    write_string(&mut out, file.name.as_bytes());
    write_compound(&mut out, &file.root);
    out
}

/// Serializes an NBT file gzip-compressed, the format `level.dat` uses (the game cannot read it uncompressed).
pub fn write_gzip(file: &NbtFile) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&write(file)).map_err(|err| err.to_string())?;
    encoder.finish().map_err(|err| err.to_string())
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "unexpected end of NBT data".to_owned())?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn len(&mut self) -> Result<usize, String> {
        let value = i32::from_be_bytes(self.array()?);
        usize::try_from(value).map_err(|_| "negative NBT length".to_owned())
    }

    fn string_bytes(&mut self) -> Result<Vec<u8>, String> {
        let len = usize::from(u16::from_be_bytes(self.array()?));
        Ok(self.take(len)?.to_vec())
    }

    fn compound(&mut self, depth: usize) -> Result<Compound, String> {
        if depth > MAX_DEPTH {
            return Err("NBT nesting too deep".to_owned());
        }
        let mut out = Vec::new();
        loop {
            let id = self.u8()?;
            if id == 0 {
                return Ok(out);
            }
            let name = String::from_utf8_lossy(&self.string_bytes()?).into_owned();
            out.push((name, self.payload(id, depth + 1)?));
        }
    }

    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag, String> {
        Ok(match id {
            1 => Tag::Byte(i8::from_be_bytes(self.array()?)),
            2 => Tag::Short(i16::from_be_bytes(self.array()?)),
            3 => Tag::Int(i32::from_be_bytes(self.array()?)),
            4 => Tag::Long(i64::from_be_bytes(self.array()?)),
            5 => Tag::Float(f32::from_be_bytes(self.array()?)),
            6 => Tag::Double(f64::from_be_bytes(self.array()?)),
            7 => {
                let len = self.len()?;
                Tag::ByteArray(self.take(len)?.iter().map(|b| *b as i8).collect())
            }
            8 => Tag::String(self.string_bytes()?),
            9 => {
                if depth > MAX_DEPTH {
                    return Err("NBT nesting too deep".to_owned());
                }
                let element_id = self.u8()?;
                let len = self.len()?;
                let mut items = Vec::new();
                for _ in 0..len {
                    items.push(self.payload(element_id, depth + 1)?);
                }
                Tag::List(TagList { element_id, items })
            }
            10 => Tag::Compound(self.compound(depth)?),
            11 => {
                let len = self.len()?;
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(i32::from_be_bytes(self.array()?));
                }
                Tag::IntArray(values)
            }
            12 => {
                let len = self.len()?;
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(i64::from_be_bytes(self.array()?));
                }
                Tag::LongArray(values)
            }
            other => return Err(format!("unknown NBT tag id {other}")),
        })
    }
}

fn write_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn write_compound(out: &mut Vec<u8>, compound: &Compound) {
    for (name, tag) in compound {
        out.push(tag.id());
        write_string(out, name.as_bytes());
        write_payload(out, tag);
    }
    out.push(0);
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            out.extend(values.iter().map(|v| *v as u8));
        }
        Tag::String(bytes) => write_string(out, bytes),
        Tag::List(list) => {
            out.push(list.element_id);
            out.extend_from_slice(&(list.items.len() as i32).to_be_bytes());
            for item in &list.items {
                write_payload(out, item);
            }
        }
        Tag::Compound(compound) => write_compound(out, compound),
        Tag::IntArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Tag::LongArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_nested_data_byte_for_byte() {
        let file = NbtFile {
            name: String::new(),
            root: vec![
                ("DataVersion".to_owned(), Tag::Int(3465)),
                (
                    "servers".to_owned(),
                    Tag::List(TagList {
                        element_id: 10,
                        items: vec![Tag::Compound(vec![
                            ("ip".to_owned(), Tag::String(b"mc.example.net".to_vec())),
                            ("hidden".to_owned(), Tag::Byte(1)),
                            ("ints".to_owned(), Tag::IntArray(vec![1, -2, 3])),
                        ])],
                    }),
                ),
                (
                    "empty".to_owned(),
                    Tag::List(TagList {
                        element_id: 8,
                        items: Vec::new(),
                    }),
                ),
            ],
        };
        let bytes = write(&file);
        assert_eq!(parse(&bytes).unwrap(), file);
        assert_eq!(write(&parse(&bytes).unwrap()), bytes);
    }

    #[test]
    fn rejects_truncated_data() {
        let bytes = write(&NbtFile {
            name: String::new(),
            root: vec![("a".to_owned(), Tag::Long(7))],
        });
        assert!(parse(&bytes[..bytes.len() - 3]).is_err());
    }
}
