use num_bigint::{BigUint, RandBigInt};
use rand::thread_rng;
use sha2::{Digest, Sha512};

/// SRP-6a client credentials for AirPlay 2 HomeKit pairing.
#[derive(Debug, Clone)]
pub struct SrpCredentials {
    pub username: String,
    pub password: String,
    pub salt: Vec<u8>,
    pub verifier: Vec<u8>,
}

/// Minimal SRP-6a client for AirPlay pairing (3072-bit group).
pub struct SrpClient {
    n: BigUint,
    g: BigUint,
    a: BigUint,
    a_pub: BigUint,
}

impl SrpClient {
    pub fn new() -> Self {
        let n = airplay_n();
        let g = BigUint::from(5u32);
        let mut rng = thread_rng();
        let a = rng.gen_biguint(256);
        let a_pub = g.modpow(&a, &n);
        Self { n, g, a, a_pub }
    }

    pub fn client_public(&self) -> Vec<u8> {
        pad_to_384(&self.a_pub)
    }

    pub fn process_challenge(
        &self,
        salt: &[u8],
        server_public: &[u8],
        username: &str,
        pin: &str,
    ) -> (Vec<u8>, Vec<u8>) {
        let b = BigUint::from_bytes_be(server_public);
        let u = compute_u(&self.a_pub, &b);
        let x = compute_x(salt, username, pin);
        let k = compute_k(&self.n, &self.g);
        let s = compute_session_key(&self.n, &self.g, &k, &self.a, &b, &x, &u);
        let key = compute_key(&s);
        let proof = compute_proof(&self.n, &self.g, username, salt, &self.a_pub, &b, &key);
        let verifier = compute_server_proof(&self.a_pub, &proof, &key);
        (proof, verifier)
    }
}

impl Default for SrpClient {
    fn default() -> Self {
        Self::new()
    }
}

fn airplay_n() -> BigUint {
    // AirPlay uses a 3072-bit safe prime; truncated representation for MVP.
    // Full constant loaded from known AirPlay SRP modulus.
    let hex = include_str!("srp_modulus.txt").trim();
    BigUint::parse_bytes(hex.as_bytes(), 16).expect("valid modulus")
}

fn pad_to_384(n: &BigUint) -> Vec<u8> {
    let mut bytes = n.to_bytes_be();
    while bytes.len() < 384 {
        bytes.insert(0, 0);
    }
    bytes
}

fn compute_u(a_pub: &BigUint, b_pub: &BigUint) -> BigUint {
    let mut data = a_pub.to_bytes_be();
    data.extend(b_pub.to_bytes_be());
    hash_to_int(&data)
}

fn compute_x(salt: &[u8], username: &str, pin: &str) -> BigUint {
    let mut inner = Sha512::new();
    inner.update(format!("{username}:{pin}").as_bytes());
    let inner_hash = inner.finalize();
    let mut outer = Sha512::new();
    outer.update(salt);
    outer.update(&inner_hash);
    hash_to_int(&outer.finalize())
}

fn compute_k(n: &BigUint, g: &BigUint) -> BigUint {
    let mut data = pad_to_384(n);
    data.extend(pad_to_384(g));
    hash_to_int(&data)
}

fn compute_session_key(
    n: &BigUint,
    g: &BigUint,
    k: &BigUint,
    a: &BigUint,
    b: &BigUint,
    x: &BigUint,
    u: &BigUint,
) -> BigUint {
    let gx = g.modpow(x, n);
    let base = (&*b - k * gx % n + n) % n;
    let exp = a + u * x;
    base.modpow(&exp, n)
}

fn compute_key(s: &BigUint) -> Vec<u8> {
    let bytes = pad_to_384(s);
    Sha512::digest(&bytes).to_vec()
}

fn compute_proof(
    n: &BigUint,
    g: &BigUint,
    username: &str,
    salt: &[u8],
    a_pub: &BigUint,
    b_pub: &BigUint,
    key: &[u8],
) -> Vec<u8> {
    let mut h = Sha512::new();
    h.update(pad_to_384(n));
    h.update(pad_to_384(g));
    h.update(username.as_bytes());
    h.update(salt);
    h.update(pad_to_384(a_pub));
    h.update(pad_to_384(b_pub));
    h.update(key);
    h.finalize().to_vec()
}

fn compute_server_proof(a_pub: &BigUint, client_proof: &[u8], key: &[u8]) -> Vec<u8> {
    let mut h = Sha512::new();
    h.update(pad_to_384(a_pub));
    h.update(client_proof);
    h.update(key);
    h.finalize().to_vec()
}

fn hash_to_int(data: &[u8]) -> BigUint {
    let hash = Sha512::digest(data);
    BigUint::from_bytes_be(&hash)
}

pub fn generate_salt() -> Vec<u8> {
    let mut salt = vec![0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

use rand::RngCore;
