//! `terms.bin`, format `FEMOSPS1` version 1 (the sparse encoding; no sparse spec ships here). Little-endian:
//! magic, `u32 version, u32 N, u64 n_one, u64 n_two, f64 threshold`, then `n_one` records
//! `u8 p, u8 q, f64 T'_pq` and `n_two` records `u8 p, q, r, s, f64 (pq|rs)`.
use super::rule::Entry;

pub const MAGIC: &[u8; 8] = b"FEMOSPS1";
const HEADER: usize = 8 + 4 + 4 + 8 + 8 + 8;

pub struct Payload {
    pub spatial_orbitals: usize,
    pub threshold: f64,
    pub n_one: usize,
    pub entries: Vec<Entry>,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

fn f64_at(b: &[u8], at: usize) -> Result<f64, String> {
    let v = f64::from_bits(u64_at(b, at));
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("terms.bin: non-finite value at byte {at}"))
    }
}

/// Parses and validates a payload: header, sizes, index ranges, canonical order and
/// uniqueness of every entry (so `sum_e w_e` is the LCU's exact 1-norm).
///
/// # Errors
/// Any malformed, out-of-range, non-canonical or repeated entry.
pub fn parse(b: &[u8]) -> Result<Payload, String> {
    if b.len() < HEADER || &b[..8] != MAGIC {
        return Err("terms.bin: bad magic or truncated header".into());
    }
    if u32_at(b, 8) != 1 {
        return Err(format!("terms.bin: unsupported version {}", u32_at(b, 8)));
    }
    let n = u32_at(b, 12) as usize;
    let n_one = usize::try_from(u64_at(b, 16)).map_err(|e| e.to_string())?;
    let n_two = usize::try_from(u64_at(b, 24)).map_err(|e| e.to_string())?;
    let threshold = f64_at(b, 32)?;
    if n == 0 || n > 255 || n_one != n * (n + 1) / 2 {
        return Err(format!("terms.bin: N = {n} with {n_one} one-body entries"));
    }
    if b.len() != HEADER + 10 * n_one + 12 * n_two {
        return Err("terms.bin: size does not match the header".into());
    }
    let mut entries = Vec::with_capacity(n_one + n_two);
    let mut at = HEADER;
    for _ in 0..n_one {
        entries.push(Entry::One {
            p: b[at],
            q: b[at + 1],
            v: f64_at(b, at + 2)?,
        });
        at += 10;
    }
    for _ in 0..n_two {
        let (p, q, r, s) = (b[at], b[at + 1], b[at + 2], b[at + 3]);
        entries.push(Entry::Two {
            p,
            q,
            r,
            s,
            v: f64_at(b, at + 4)?,
        });
        at += 12;
    }
    check_canonical(&entries, n)?;
    Ok(Payload {
        spatial_orbitals: n,
        threshold,
        n_one,
        entries,
    })
}

fn key(e: &Entry) -> (u8, [u8; 4]) {
    match *e {
        Entry::One { p, q, .. } => (0, [p, q, 0, 0]),
        Entry::Two { p, q, r, s, .. } => (1, [p, q, r, s]),
    }
}

fn check_canonical(entries: &[Entry], n: usize) -> Result<(), String> {
    for (i, e) in entries.iter().enumerate() {
        let ok = match *e {
            Entry::One { p, q, .. } => p <= q && usize::from(q) < n,
            Entry::Two { p, q, r, s, .. } => {
                p <= q && r <= s && (p, q) <= (r, s) && usize::from(q.max(s)) < n
            }
        };
        if !ok {
            return Err(format!(
                "terms.bin: entry {i} out of range or not canonical"
            ));
        }
        if i > 0 && key(&entries[i - 1]) >= key(e) {
            return Err(format!("terms.bin: entry {i} not strictly increasing"));
        }
    }
    Ok(())
}
