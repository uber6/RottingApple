use std::collections::HashMap;

/// TLV8 type tags used in HomeKit / AirPlay 2 pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TlvType {
    Method = 0x00,
    Identifier = 0x01,
    Salt = 0x02,
    PublicKey = 0x03,
    Proof = 0x04,
    EncryptedData = 0x05,
    State = 0x06,
    Error = 0x07,
    RetryDelay = 0x08,
    Certificate = 0x09,
    Signature = 0x0A,
    Permissions = 0x0B,
    FragmentData = 0x0C,
    FragmentLast = 0x0D,
    Separator = 0xFF,
}

impl From<TlvType> for u8 {
    fn from(t: TlvType) -> u8 {
        t as u8
    }
}

/// Encode a map of TLV entries to bytes (TLV8 format).
pub fn encode(entries: &[(TlvType, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (typ, value) in entries {
        let t: u8 = (*typ).into();
        for chunk in value.chunks(255) {
            out.push(t);
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
    }
    out
}

/// Decode TLV8 bytes into a map (concatenates fragmented values).
pub fn decode(data: &[u8]) -> HashMap<u8, Vec<u8>> {
    let mut map: HashMap<u8, Vec<u8>> = HashMap::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let typ = data[i];
        let len = data[i + 1] as usize;
        i += 2;
        if i + len > data.len() {
            break;
        }
        map.entry(typ)
            .or_default()
            .extend_from_slice(&data[i..i + len]);
        i += len;
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let entries = [
            (TlvType::State, &[1u8][..]),
            (TlvType::Method, &[0u8, 1u8][..]),
        ];
        let encoded = encode(&entries);
        let decoded = decode(&encoded);
        assert_eq!(decoded.get(&6), Some(&vec![1]));
        assert_eq!(decoded.get(&0), Some(&vec![0, 1]));
    }
}
