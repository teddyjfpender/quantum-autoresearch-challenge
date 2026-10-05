//! `thc.bin` (format `FEMOTHC1`, version 1), little-endian:
//! `magic, u32 version, u32 N, u32 beta, u32 M, f64 ecore, f64 t[N], u32 t_angles[N][N-1],
//! u32 chi_angles[M][N-1], f64 zeta[M (M + 1) / 2]`. Angles of one network are the rotations
//! `(j, j+1)` for `j = 0..N-1` in order (the df convention). `zeta` is in pair order: for
//! `nu = 0..M`, `mu = 0..=nu` (the THC encoding; no THC spec ships here).
use super::ThcSpec;
use crate::spec::Network;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"FEMOTHC1";

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).ok_or("thc.bin: truncated")?;
        let s = self.b.get(self.at..end).ok_or("thc.bin: truncated")?;
        self.at = end;
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
            return Err("thc.bin: non-finite value".into());
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
                    return Err(format!("thc.bin: angle {a} has more than {beta} bits"));
                }
                let j = u16::try_from(j).map_err(|e| e.to_string())?;
                rotations.push((j, j + 1, a));
            }
            out.push(Arc::new(Network { beta, rotations }));
        }
        Ok(out)
    }
}

/// Parses and checks a payload; `lambda`, `lambda_lanes` and `identity` are recomputed exactly.
///
/// # Errors
/// Bad magic or version, out-of-range sizes, non-finite values, trailing bytes.
pub fn parse_payload(id: &str, bytes: &[u8]) -> Result<ThcSpec, String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err("thc.bin: bad magic".into());
    }
    let (version, n, beta, m) = (r.u32()?, r.u32()? as usize, r.u32()?, r.u32()? as usize);
    if version != 1
        || !(2..=1024).contains(&n)
        || !(3..=32).contains(&beta)
        || !(1..=1 << 16).contains(&m)
    {
        return Err(format!(
            "thc.bin: version {version}, N {n}, beta {beta}, M {m} not supported"
        ));
    }
    let beta = u8::try_from(beta).map_err(|e| e.to_string())?;
    let ecore = r.f64s(1)?[0];
    let t = r.f64s(n)?;
    let t_nets = r.networks(n, n, beta)?;
    let chi_nets = r.networks(m, n, beta)?;
    let zeta_tri = r.f64s(m * (m + 1) / 2)?;
    if r.at != bytes.len() {
        return Err("thc.bin: trailing bytes".into());
    }
    let (lambda, lambda_lanes, identity) = ThcSpec::exact_lambda_identity(ecore, &t, &zeta_tri);
    Ok(ThcSpec {
        id: id.to_string(),
        n,
        m,
        beta,
        ecore,
        t,
        t_nets,
        chi_nets,
        zeta_tri,
        lambda,
        lambda_lanes,
        identity,
        sha: Sha256::digest(bytes).into(),
    })
}
