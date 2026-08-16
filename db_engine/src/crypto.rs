// db_engine/src/crypto.rs
// XChaCha20-Poly1305 AEAD

const SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

macro_rules! qr {
    ($s:expr, $a:expr, $b:expr, $c:expr, $d:expr) => {{
        $s[$a]=$s[$a].wrapping_add($s[$b]); $s[$d]^=$s[$a]; $s[$d]=$s[$d].rotate_left(16);
        $s[$c]=$s[$c].wrapping_add($s[$d]); $s[$b]^=$s[$c]; $s[$b]=$s[$b].rotate_left(12);
        $s[$a]=$s[$a].wrapping_add($s[$b]); $s[$d]^=$s[$a]; $s[$d]=$s[$d].rotate_left(8);
        $s[$c]=$s[$c].wrapping_add($s[$d]); $s[$b]^=$s[$c]; $s[$b]=$s[$b].rotate_left(7);
    }};
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[0]=SIGMA[0]; s[1]=SIGMA[1]; s[2]=SIGMA[2]; s[3]=SIGMA[3];
    for i in 0..8 { s[4+i] = u32::from_le_bytes(key[i*4..i*4+4].try_into().unwrap()); }
    s[12] = counter;
    for i in 0..3 { s[13+i] = u32::from_le_bytes(nonce[i*4..i*4+4].try_into().unwrap()); }
    let init = s;
    for _ in 0..10 {
        qr!(s,0,4,8,12); qr!(s,1,5,9,13); qr!(s,2,6,10,14); qr!(s,3,7,11,15);
        qr!(s,0,5,10,15); qr!(s,1,6,11,12); qr!(s,2,7,8,13); qr!(s,3,4,9,14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 { out[i*4..i*4+4].copy_from_slice(&s[i].wrapping_add(init[i]).to_le_bytes()); }
    out
}

pub fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], buf: &mut [u8]) {
    let mut pos = 0usize; let mut ctr = counter;
    while pos < buf.len() {
        let block = chacha20_block(key, ctr, nonce);
        let n = (buf.len() - pos).min(64);
        for i in 0..n { buf[pos + i] ^= block[i]; }
        pos += n; ctr = ctr.wrapping_add(1);
    }
}

pub fn hchacha20(key: &[u8; 32], nonce16: &[u8; 16]) -> [u8; 32] {
    let mut s = [0u32; 16];
    s[0]=SIGMA[0]; s[1]=SIGMA[1]; s[2]=SIGMA[2]; s[3]=SIGMA[3];
    for i in 0..8  { s[4+i]  = u32::from_le_bytes(key[i*4..i*4+4].try_into().unwrap()); }
    for i in 0..4  { s[12+i] = u32::from_le_bytes(nonce16[i*4..i*4+4].try_into().unwrap()); }
    for _ in 0..10 {
        qr!(s,0,4,8,12); qr!(s,1,5,9,13); qr!(s,2,6,10,14); qr!(s,3,7,11,15);
        qr!(s,0,5,10,15); qr!(s,1,6,11,12); qr!(s,2,7,8,13); qr!(s,3,4,9,14);
    }
    let mut out = [0u8; 32];
    for i in 0..4   { out[i*4..i*4+4].copy_from_slice(&s[i].to_le_bytes()); }
    for i in 12..16 { out[(i-8)*4..(i-8)*4+4].copy_from_slice(&s[i].to_le_bytes()); }
    out
}

pub fn derive_key(master: &[u8; 32], salt: &[u8; 16]) -> [u8; 32] {
    hchacha20(master, salt)
}

fn poly1305_mac(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let mut r = [0u8; 16];
    r.copy_from_slice(&key[0..16]);
    let s = u128::from_le_bytes(key[16..32].try_into().unwrap());
    r[3]&=15; r[7]&=15; r[11]&=15; r[15]&=15;
    r[4]&=252; r[8]&=252; r[12]&=252;
    let r128 = u128::from_le_bytes(r);
    let mut acc: u128 = 0;
    for chunk in msg.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        acc = acc.wrapping_add(u128::from_le_bytes(block));
        acc = acc.wrapping_mul(r128);
    }
    acc = acc.wrapping_add(s);
    acc.to_le_bytes()[..16].try_into().unwrap()
}

#[derive(Debug, Clone, PartialEq)]
pub enum CryptoError { AuthTagMismatch, InvalidLength }
impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::AuthTagMismatch => write!(f, "Authentication tag mismatch: data corrupted or tampered"),
            CryptoError::InvalidLength   => write!(f, "Invalid ciphertext length"),
        }
    }
}
impl std::error::Error for CryptoError {}

pub fn xchacha20poly1305_encrypt(
    key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], pt: &[u8],
) -> Vec<u8> {
    let subkey = hchacha20(key, nonce[0..16].try_into().unwrap());
    let mut n12 = [0u8; 12]; n12[4..12].copy_from_slice(&nonce[16..24]);
    let ks0 = chacha20_block(&subkey, 0, &n12);
    let mut pk = [0u8; 32]; pk.copy_from_slice(&ks0[..32]);
    let mut ct = pt.to_vec();
    chacha20_xor(&subkey, 1, &n12, &mut ct);
    let tag = poly1305_mac(&pk, &mac_input(aad, &ct));
    ct.extend_from_slice(&tag); ct
}

pub fn xchacha20poly1305_decrypt(
    key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], ct_tag: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if ct_tag.len() < 16 { return Err(CryptoError::InvalidLength); }
    let (ct, tag) = ct_tag.split_at(ct_tag.len() - 16);
    let subkey = hchacha20(key, nonce[0..16].try_into().unwrap());
    let mut n12 = [0u8; 12]; n12[4..12].copy_from_slice(&nonce[16..24]);
    let ks0 = chacha20_block(&subkey, 0, &n12);
    let mut pk = [0u8; 32]; pk.copy_from_slice(&ks0[..32]);
    let expected = poly1305_mac(&pk, &mac_input(aad, ct));
    if tag != expected.as_slice() { return Err(CryptoError::AuthTagMismatch); }
    let mut pt = ct.to_vec();
    chacha20_xor(&subkey, 1, &n12, &mut pt);
    Ok(pt)
}

fn mac_input(aad: &[u8], ct: &[u8]) -> Vec<u8> {
    let pad = |n: usize| vec![0u8; (16 - n % 16) % 16];
    let mut v = aad.to_vec(); v.extend(pad(aad.len()));
    v.extend_from_slice(ct);  v.extend(pad(ct.len()));
    v.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    v.extend_from_slice(&(ct.len() as u64).to_le_bytes()); v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_roundtrip() {
        let k=[0x42u8;32]; let n=[0x24u8;24];
        let pt=b"KAGURA secret data";
        let ct=xchacha20poly1305_encrypt(&k,&n,b"aad",pt);
        assert_ne!(ct.as_slice(),pt.as_slice());
        let dec=xchacha20poly1305_decrypt(&k,&n,b"aad",&ct).unwrap();
        assert_eq!(dec,pt);
    }
    #[test]
    fn test_tamper() {
        let k=[0x11u8;32]; let n=[0x22u8;24];
        let mut ct=xchacha20poly1305_encrypt(&k,&n,b"hdr",b"data");
        ct[0]^=1;
        assert_eq!(xchacha20poly1305_decrypt(&k,&n,b"hdr",&ct),Err(CryptoError::AuthTagMismatch));
    }
    #[test]
    fn test_chacha20_xor() {
        let k=[0u8;32]; let n=[0u8;12];
        let mut d=b"Hello!".to_vec(); let o=d.clone();
        chacha20_xor(&k,0,&n,&mut d); assert_ne!(d,o);
        chacha20_xor(&k,0,&n,&mut d); assert_eq!(d,o);
    }
}
