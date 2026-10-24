//! Just enough protobuf wire format for the collection service, whose
//! messages (`collection2v2.proto`) librespot doesn't compile in.

pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.0.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.0.push(v as u8);
    }

    pub fn string(mut self, field: u32, s: &str) -> Self {
        self.bytes_field(field, s.as_bytes());
        self
    }

    pub fn int(mut self, field: u32, v: u64) -> Self {
        self.varint((field << 3) as u64);
        self.varint(v);
        self
    }

    pub fn message(mut self, field: u32, m: Writer) -> Self {
        self.bytes_field(field, &m.0);
        self
    }

    fn bytes_field(&mut self, field: u32, b: &[u8]) {
        self.varint(((field << 3) | 2) as u64);
        self.varint(b.len() as u64);
        self.0.extend_from_slice(b);
    }
}

pub enum Field<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

/// All top-level fields of a message, in wire order.
pub fn fields(b: &[u8]) -> Vec<(u32, Field<'_>)> {
    fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let x = *b.get(*i)?;
            *i += 1;
            v |= ((x & 0x7f) as u64) << shift;
            if x < 0x80 {
                return Some(v);
            }
        }
        None
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(key) = varint(b, &mut i) else { break };
        let field = (key >> 3) as u32;
        match key & 7 {
            0 => match varint(b, &mut i) {
                Some(v) => out.push((field, Field::Int(v))),
                None => break,
            },
            2 => {
                let Some(n) = varint(b, &mut i) else { break };
                let Some(end) = usize::try_from(n).ok().and_then(|n| i.checked_add(n)) else { break };
                let Some(slice) = b.get(i..end) else { break };
                out.push((field, Field::Bytes(slice)));
                i = end;
            }
            1 => i += 8,
            5 => i += 4,
            _ => break,
        }
    }
    out
}

pub fn string(f: &Field) -> String {
    match f {
        Field::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        Field::Int(_) => String::new(),
    }
}

pub fn int(f: &Field) -> u64 {
    match f {
        Field::Int(v) => *v,
        Field::Bytes(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let inner = Writer::new().string(1, "spotify:track:1").int(2, 1700000000).int(3, 1);
        let msg = Writer::new().string(1, "bob").message(3, inner).string(4, "x");
        let f = fields(&msg.0);
        assert_eq!(string(&f[0].1), "bob");
        let Field::Bytes(b) = f[1].1 else { panic!() };
        let inner = fields(b);
        assert_eq!(string(&inner[0].1), "spotify:track:1");
        assert_eq!(int(&inner[1].1), 1700000000);
        assert_eq!(int(&inner[2].1), 1);
        assert_eq!(f[2].0, 4);
    }

    #[test]
    fn truncated_input_stops_cleanly() {
        let msg = Writer::new().string(1, "hello").0;
        assert!(fields(&msg[..msg.len() - 2]).is_empty());
    }
}
