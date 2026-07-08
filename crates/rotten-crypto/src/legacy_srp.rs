use num_bigint::BigUint;
use num_traits::Zero;
use rand::RngCore;
use sha1::{Digest, Sha1};

/// Apple legacy AirPlay pair-setup-pin SRP client (RFC 5054 NG_2048, SHA-1).
/// Matches AirPlayAuth / nimbus AppleSRP6ClientSessionImpl.
pub struct LegacySrpClient {
    n: BigUint,
    g: BigUint,
    a: BigUint,
    a_nat: Vec<u8>,
    b_nat: Vec<u8>,
    username: String,
    salt: Vec<u8>,
    session_key: Option<Vec<u8>>,
}

impl LegacySrpClient {
    pub fn new(salt: &[u8], server_pk: &[u8], username: &str) -> Self {
        let n = ng_2048();
        let g = BigUint::from(2u32);
        let mut a_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut a_bytes);
        let a = BigUint::from_bytes_be(&a_bytes);
        let a_pub = g.modpow(&a, &n);
        let a_nat = a_pub.to_bytes_be();
        let b_pub = BigUint::from_bytes_be(server_pk);
        let b_nat = b_pub.to_bytes_be();
        Self {
            n,
            g,
            a,
            a_nat,
            b_nat,
            username: username.to_string(),
            salt: salt.to_vec(),
            session_key: None,
        }
    }

    /// Client SRP public key (A) for pair-setup-pin plist — natural big-endian bytes.
    pub fn client_public(&self) -> Vec<u8> {
        self.a_nat.clone()
    }

    pub fn authenticate(&mut self, pin: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        let x = self.compute_x(pin);
        let u = self.compute_u();
        let k = self.compute_k_multiplier();
        let b_pub = BigUint::from_bytes_be(&self.b_nat);
        let s = self.compute_session_s(&x, &u, &k, &b_pub)?;
        let session_key = apple_session_key_hash(&s);
        self.session_key = Some(session_key.clone());

        let m1 = self.compute_m1(&session_key);
        let m2 = self.compute_m2(&m1, &session_key);
        Ok((m1, m2))
    }

    pub fn session_key_hash(&self) -> Option<&[u8]> {
        self.session_key.as_deref()
    }

    fn compute_x(&self, pin: &str) -> BigUint {
        let inner = format!("{}:{}", self.username, pin);
        let inner_hash = sha1_bytes(inner.as_bytes());
        let mut data = self.salt.clone();
        data.extend_from_slice(&inner_hash);
        bytes_to_int(&sha1_bytes(&data))
    }

    fn compute_u(&self) -> BigUint {
        let mut data = self.a_nat.clone();
        data.extend_from_slice(&self.b_nat);
        bytes_to_int(&sha1_bytes(&data))
    }

    fn compute_k_multiplier(&self) -> BigUint {
        let data = [
            pad_to_256(&self.n.to_bytes_be()),
            pad_to_256(&self.g.to_bytes_be()),
        ]
        .concat();
        bytes_to_int(&sha1_bytes(&data))
    }

    fn compute_session_s(
        &self,
        x: &BigUint,
        u: &BigUint,
        k: &BigUint,
        b_pub: &BigUint,
    ) -> Result<BigUint, String> {
        let gx = self.g.modpow(x, &self.n);
        let kgx = (k * &gx) % &self.n;
        let mut base = if b_pub >= &kgx {
            b_pub - &kgx
        } else {
            b_pub + &self.n - &kgx
        };
        base %= &self.n;
        if base.is_zero() {
            return Err("invalid server public key".into());
        }
        let exp = &self.a + (u * x);
        Ok(base.modpow(&exp, &self.n))
    }

    /// M1 = H(H(N) xor H(g) || H(username) || salt || A || B || K_apple)
    fn compute_m1(&self, session_key: &[u8]) -> Vec<u8> {
        let hn = sha1_bytes(&self.n.to_bytes_be());
        let hg = sha1_bytes(&self.g.to_bytes_be());
        let hxor: Vec<u8> = hn.iter().zip(hg.iter()).map(|(a, b)| a ^ b).collect();
        let hu = sha1_bytes(self.username.as_bytes());

        let mut data = hxor;
        data.extend_from_slice(&hu);
        data.extend_from_slice(&self.salt);
        data.extend_from_slice(&self.a_nat);
        data.extend_from_slice(&self.b_nat);
        data.extend_from_slice(session_key);
        sha1_bytes(&data).to_vec()
    }

    /// M2 = H(A || M1 || K_apple)
    fn compute_m2(&self, m1: &[u8], session_key: &[u8]) -> Vec<u8> {
        let mut data = self.a_nat.clone();
        data.extend_from_slice(m1);
        data.extend_from_slice(session_key);
        sha1_bytes(&data).to_vec()
    }
}

fn apple_session_key_hash(s: &BigUint) -> Vec<u8> {
    let s_bytes = s.to_bytes_be();
    let mut k1_input = s_bytes.clone();
    k1_input.extend_from_slice(&[0, 0, 0, 0]);
    let k1 = sha1_bytes(&k1_input);
    let mut k2_input = s_bytes;
    k2_input.extend_from_slice(&[0, 0, 0, 1]);
    let k2 = sha1_bytes(&k2_input);
    [k1.as_slice(), k2.as_slice()].concat()
}

fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn bytes_to_int(bytes: &[u8]) -> BigUint {
    BigUint::from_bytes_be(bytes)
}

fn pad_to_256(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 256];
    let len = bytes.len().min(256);
    let start = 256 - len;
    out[start..].copy_from_slice(&bytes[bytes.len() - len..]);
    out
}

fn ng_2048() -> BigUint {
    let hex = concat!(
        "AC6BDB41324A9A9BF166DE5E1389582F",
        "AF72B6651987EE07FC3192943DB56050",
        "A37329CBB4A099ED8193E0757767A13D",
        "D52312AB4B03310DCD7F48A9DA04FD50",
        "E8083969EDB767B0CF6095179A163AB3",
        "661A05FBD5FAAAE82918A9962F0B93B8",
        "55F97993EC975EEAA80D740ADBF4FF74",
        "7359D041D5C33EA71D281E446B14773B",
        "CA97B43A23FB801676BD207A436C6481",
        "F1D2B9078717461A5B9D32E688F87748",
        "544523B524B0D57D5EA77A2775D2ECFA",
        "032CFBDBF52FB3786160279004E57AE6",
        "AF874E7303CE53299CCC041C7BC308D8",
        "2A5698F3A8D0C38271AE35F8E9DBFBB6",
        "94B5C803D89F7AE435DE236D525F5475",
        "9B65E372FCD68EF20FA7111F9E4AFF73"
    );
    debug_assert_eq!(hex.len(), 512);
    BigUint::parse_bytes(hex.as_bytes(), 16).expect("valid NG_2048")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ng_2048_is_256_bytes() {
        assert_eq!(ng_2048().to_bytes_be().len(), 256);
    }
}
