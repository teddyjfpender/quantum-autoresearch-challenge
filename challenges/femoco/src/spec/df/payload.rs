//! `df.bin` (format `FEMODFS1`, version 1), little-endian:
//! `magic, u32 version, u32 N, u32 beta, u32 L, f64 ecore, f64 t[N], u32 t_angles[N][N-1]`,
//! then per leaf `u32 Xi, f64 e[Xi], u32 angles[Xi][N-1]`. Angles of one network are the
//! rotations `(j, j+1)` for `j = 0..N-1` in order (the DF encoding; no DF spec ships here).
use super::{DfSpec, Leaf};
use crate::spec::Network;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"FEMODFS1";

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or("df.bin: truncated")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64s(&mut self, n: usize) -> Result<Vec<f64>, String> {
        let s = self.take(8 * n)?;
        let v: Vec<f64> = s
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
            .collect();
        if v.iter().any(|x| !x.is_finite()) {
            return Err("df.bin: non-finite value".into());
        }
        Ok(v)
    }
    fn networks(&mut self, count: usize, n: usize, beta: u8) -> Result<Vec<Arc<Network>>, String> {
        let s = self.take(4 * count * (n - 1))?;
        let mut out = Vec::with_capacity(count);
        for net in s.chunks_exact(4 * (n - 1)) {
            let mut rotations = Vec::with_capacity(n - 1);
            for (j, c) in net.chunks_exact(4).enumerate() {
                let a = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                if u64::from(a) >> beta != 0 {
                    return Err(format!("df.bin: angle {a} has more than {beta} bits"));
                }
                let j = u16::try_from(j).map_err(|e| e.to_string())?;
                rotations.push((j, j + 1, a));
            }
            out.push(Arc::new(Network { beta, rotations }));
        }
        Ok(out)
    }
}

/// Parses and checks a payload; `lambda` and `identity` are recomputed exactly.
///
/// # Errors
/// Bad magic or version, out-of-range sizes, non-finite values, trailing bytes.
pub fn parse_payload(id: &str, bytes: &[u8]) -> Result<DfSpec, String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err("df.bin: bad magic".into());
    }
    let (version, n, beta, l) = (r.u32()?, r.u32()? as usize, r.u32()?, r.u32()? as usize);
    if version != 1 || !(2..=1024).contains(&n) || !(3..=32).contains(&beta) {
        return Err(format!(
            "df.bin: version {version}, N {n}, beta {beta} not supported"
        ));
    }
    let beta = u8::try_from(beta).map_err(|e| e.to_string())?;
    let ecore = r.f64s(1)?[0];
    let t = r.f64s(n)?;
    let t_nets = r.networks(n, n, beta)?;
    let mut leaves = Vec::with_capacity(l.min(1 << 16));
    let mut offsets = vec![n as u64];
    for _ in 0..l {
        let xi = r.u32()? as usize;
        if !(1..=n).contains(&xi) {
            return Err(format!("df.bin: leaf with {xi} eigenvectors"));
        }
        let e = r.f64s(xi)?;
        let nets = r.networks(xi, n, beta)?;
        offsets.push(offsets.last().copied().unwrap_or(0) + (xi * xi) as u64);
        leaves.push(Leaf { e, nets });
    }
    if r.at != bytes.len() {
        return Err("df.bin: trailing bytes".into());
    }
    let (lambda, identity) = DfSpec::exact_lambda_identity(ecore, &t, &leaves);
    Ok(DfSpec {
        id: id.to_string(),
        n,
        beta,
        ecore,
        t,
        t_nets,
        leaves,
        offsets,
        lambda,
        identity,
        sha: Sha256::digest(bytes).into(),
        form: super::DfForm::Flat,
    })
}

/// When `specs/INDEX.json` exists next to the spec directory, its entry for `id` must carry the
/// same payload hash.
///
/// # Errors
/// An entry with a different hash, or an index that does not parse.
pub fn check_index(dir: &Path, id: &str, sha: &str) -> Result<(), String> {
    let Some(index) = dir.parent().map(|p| p.join("INDEX.json")) else {
        return Ok(());
    };
    let Ok(text) = std::fs::read_to_string(&index) else {
        return Ok(());
    };
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("INDEX.json: {e}"))?;
    let entry = v
        .get("specs")
        .and_then(serde_json::Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|s| s.get("id").and_then(|x| x.as_str()) == Some(id))
        });
    match entry
        .and_then(|e| e.get("payload_sha256"))
        .and_then(|x| x.as_str())
    {
        Some(h) if h != sha => Err(format!(
            "spec {id}: INDEX.json payload sha256 {h} differs from the payload's {sha}"
        )),
        _ => Ok(()),
    }
}
