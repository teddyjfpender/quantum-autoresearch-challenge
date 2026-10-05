//! `ops.bin`: magic `FEMOOPS1`, `u64` op count, then 56 bytes per op in ecdsa.fail's layout
//! (`u32 kind, u32 pad, u64 q_control2, u64 q_control1, u64 q_target, u64 c_target,
//! u64 c_condition, u64 r_target`, little-endian).
use super::{Op, OperationType, NONE};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"FEMOOPS1";
pub const OP_BYTES: usize = 56;
/// Sanity cap so a forged header cannot make the reader allocate without bound.
pub const MAX_OPS: u64 = 2_000_000_000;

fn encode(op: &Op) -> [u8; OP_BYTES] {
    let mut b = [0u8; OP_BYTES];
    b[..4].copy_from_slice(&(op.kind as u32).to_le_bytes());
    let fields = [
        op.q_control2,
        op.q_control1,
        op.q_target,
        op.c_target,
        op.c_condition,
        op.r_target,
    ];
    for (i, f) in fields.into_iter().enumerate() {
        let v = if f == NONE { u64::MAX } else { u64::from(f) };
        b[8 + 8 * i..16 + 8 * i].copy_from_slice(&v.to_le_bytes());
    }
    b
}

fn decode(b: &[u8; OP_BYTES], index: u64) -> Result<Op, String> {
    let raw = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let kind =
        OperationType::from_u32(raw).ok_or_else(|| format!("op {index}: unknown kind {raw}"))?;
    if b[4..8] != [0; 4] {
        return Err(format!("op {index}: nonzero padding"));
    }
    let mut f = [NONE; 6];
    for (i, slot) in f.iter_mut().enumerate() {
        let mut w = [0u8; 8];
        w.copy_from_slice(&b[8 + 8 * i..16 + 8 * i]);
        let v = u64::from_le_bytes(w);
        *slot = if v == u64::MAX {
            NONE
        } else {
            u32::try_from(v)
                .ok()
                .filter(|&x| x != NONE)
                .ok_or_else(|| format!("op {index}: operand {v} out of range"))?
        };
    }
    let op = Op {
        kind,
        q_control2: f[0],
        q_control1: f[1],
        q_target: f[2],
        c_target: f[3],
        c_condition: f[4],
        r_target: f[5],
    };
    op.validate().map_err(|e| format!("op {index}: {e}"))?;
    Ok(op)
}

/// Writes `ops` to `path` atomically (temp file then rename).
///
/// # Errors
/// Any I/O error.
pub fn write_ops(ops: &[Op], path: &Path) -> std::io::Result<()> {
    let tmp = path.with_extension("bin.tmp");
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        w.write_all(MAGIC)?;
        w.write_all(&(ops.len() as u64).to_le_bytes())?;
        for op in ops {
            w.write_all(&encode(op))?;
        }
        w.flush()?;
    }
    std::fs::rename(&tmp, path)
}

/// A loaded op stream and the SHA-256 of its exact bytes.
pub struct OpsFile {
    pub ops: Vec<Op>,
    pub sha256: [u8; 32],
}

/// Streams `ops.bin`, validating every op's kind, padding and operand shape, and hashing the
/// bytes as they are read. Proves: the file is exactly `header + count * 56` bytes and every
/// op is well formed.
///
/// # Errors
/// I/O errors, a bad header or length, or the first malformed op.
pub fn read_ops(path: &Path) -> Result<OpsFile, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut hasher = Sha256::new();
    let mut head = [0u8; 16];
    r.read_exact(&mut head).map_err(|_| "ops.bin: too short")?;
    hasher.update(head);
    if &head[..8] != MAGIC {
        return Err("ops.bin: bad magic (expected FEMOOPS1)".into());
    }
    let n = u64::from_le_bytes(head[8..].try_into().map_err(|_| "ops.bin: header")?);
    if n > MAX_OPS {
        return Err(format!("ops.bin: op count {n} exceeds cap {MAX_OPS}"));
    }
    if len != 16 + n * OP_BYTES as u64 {
        return Err(format!("ops.bin: length {len} does not match {n} ops"));
    }
    let mut ops = Vec::with_capacity(usize::try_from(n).map_err(|e| e.to_string())?);
    let mut b = [0u8; OP_BYTES];
    for i in 0..n {
        r.read_exact(&mut b).map_err(|e| format!("ops.bin: {e}"))?;
        hasher.update(b);
        ops.push(decode(&b, i)?);
    }
    Ok(OpsFile {
        ops,
        sha256: hasher.finalize().into(),
    })
}
