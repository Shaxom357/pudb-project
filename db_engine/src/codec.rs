// db_engine/src/codec.rs
// KAGURA DB バイナリシリアライズ（独自フォーマット）
//
// レコード1件のバイナリ構造:
//   [u64 id][u16 label_count]([u16 len][bytes label])*
//   [u16 col_count]([u16 key_len][bytes key][u8 type_tag][data])*
//
// type_tag: 0x01=Text(u32+bytes) 0x02=Integer(i64) 0x03=Float(f64)
//           0x04=Boolean(u8)     0x05=Null

use std::collections::HashMap;
use crate::{DataType, Record};

#[derive(Debug)]
pub enum CodecError {
    UnexpectedEof,
    InvalidTypeTag(u8),
    InvalidUtf8,
}
impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::UnexpectedEof     => write!(f, "Unexpected EOF"),
            CodecError::InvalidTypeTag(t) => write!(f, "Invalid type tag: 0x{:02X}", t),
            CodecError::InvalidUtf8       => write!(f, "Invalid UTF-8"),
        }
    }
}
impl std::error::Error for CodecError {}

pub fn encode_record(r: &Record) -> Vec<u8> {
    let mut b = Vec::with_capacity(256);
    b.extend_from_slice(&r.id.to_le_bytes());
    b.extend_from_slice(&(r.labels.len() as u16).to_le_bytes());
    for lbl in &r.labels {
        let lb = lbl.as_bytes();
        b.extend_from_slice(&(lb.len() as u16).to_le_bytes());
        b.extend_from_slice(lb);
    }
    b.extend_from_slice(&(r.columns.len() as u16).to_le_bytes());
    for (key, val) in &r.columns {
        let kb = key.as_bytes();
        b.extend_from_slice(&(kb.len() as u16).to_le_bytes());
        b.extend_from_slice(kb);
        match val {
            DataType::Text(s)    => { b.push(0x01); let sb=s.as_bytes(); b.extend_from_slice(&(sb.len() as u32).to_le_bytes()); b.extend_from_slice(sb); }
            DataType::Integer(n) => { b.push(0x02); b.extend_from_slice(&n.to_le_bytes()); }
            DataType::Float(f)   => { b.push(0x03); b.extend_from_slice(&f.to_le_bytes()); }
            DataType::Boolean(v) => { b.push(0x04); b.push(if *v {1} else {0}); }
            DataType::Null       => { b.push(0x05); }
        }
    }
    b
}

pub fn decode_record(src: &[u8]) -> Result<Record, CodecError> {
    let mut p = 0usize;
    let id   = rd_u64(src,&mut p)?;
    let nlbl = rd_u16(src,&mut p)? as usize;
    let mut labels = Vec::with_capacity(nlbl);
    for _ in 0..nlbl { labels.push(rd_str16(src,&mut p)?); }
    let ncol = rd_u16(src,&mut p)? as usize;
    let mut columns = HashMap::with_capacity(ncol);
    for _ in 0..ncol {
        let key = rd_str16(src,&mut p)?;
        let val = match rd_u8(src,&mut p)? {
            0x01 => { let l=rd_u32(src,&mut p)? as usize; DataType::Text(rd_string(src,&mut p,l)?) }
            0x02 => DataType::Integer(i64::from_le_bytes(rd_bytes(src,&mut p,8)?.try_into().unwrap())),
            0x03 => DataType::Float(f64::from_le_bytes(rd_bytes(src,&mut p,8)?.try_into().unwrap())),
            0x04 => DataType::Boolean(rd_u8(src,&mut p)? != 0),
            0x05 => DataType::Null,
            t    => return Err(CodecError::InvalidTypeTag(t)),
        };
        columns.insert(key, val);
    }
    Ok(Record { id, columns, labels })
}

fn rd_bytes<'a>(src: &'a [u8], p: &mut usize, n: usize) -> Result<&'a [u8], CodecError> {
    if *p + n > src.len() { return Err(CodecError::UnexpectedEof); }
    let s = &src[*p..*p+n]; *p += n; Ok(s)
}
fn rd_u8(src:&[u8],p:&mut usize)->Result<u8,CodecError>    { Ok(rd_bytes(src,p,1)?[0]) }
fn rd_u16(src:&[u8],p:&mut usize)->Result<u16,CodecError>   { Ok(u16::from_le_bytes(rd_bytes(src,p,2)?.try_into().unwrap())) }
fn rd_u32(src:&[u8],p:&mut usize)->Result<u32,CodecError>   { Ok(u32::from_le_bytes(rd_bytes(src,p,4)?.try_into().unwrap())) }
fn rd_u64(src:&[u8],p:&mut usize)->Result<u64,CodecError>   { Ok(u64::from_le_bytes(rd_bytes(src,p,8)?.try_into().unwrap())) }
fn rd_str16(src:&[u8],p:&mut usize)->Result<String,CodecError> {
    let l = rd_u16(src,p)? as usize; rd_string(src,p,l)
}
fn rd_string(src:&[u8],p:&mut usize,l:usize)->Result<String,CodecError> {
    std::str::from_utf8(rd_bytes(src,p,l)?)
        .map(|s|s.to_string()).map_err(|_|CodecError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_roundtrip() {
        let mut r = Record::new(99);
        r.set("name", DataType::Text("田中".into()));
        r.set("age",  DataType::Integer(35));
        r.set("rate", DataType::Float(4.5));
        r.set("ok",   DataType::Boolean(true));
        r.set("memo", DataType::Null);
        r.add_label("employee"); r.add_label("dept:eng");
        let d = decode_record(&encode_record(&r)).unwrap();
        assert_eq!(d.id, 99);
        assert_eq!(d.columns.get("name"), Some(&DataType::Text("田中".into())));
        assert_eq!(d.columns.get("age"),  Some(&DataType::Integer(35)));
        assert_eq!(d.columns.get("memo"), Some(&DataType::Null));
        assert!(d.labels.contains(&"employee".to_string()));
    }
}
