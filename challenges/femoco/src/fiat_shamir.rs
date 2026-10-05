//! Fiat-Shamir sampling (spec/DESIGN.md section 9).
//!
//! Seed: `SHAKE256("femoco-walk-fiat-shamir-v1" || sha256(spec payload) || sha256(lanemap.bin)
//! || sha256(family.out.json) || sha256(ops.bin))`. From that XOF, in order:
//! 1. 32 bytes: the measurement key, from which each batch's `Hmr` outcomes are drawn
//!    (`SHAKE256("femoco-walk-hmr-v1" || key || u64 batch index)`, read `8 W` bytes per
//!    `Hmr` op the batch reaches, in op order), so outcomes do not depend on thread scheduling;
//! 2. one little-endian `u64` per lane: control bit `c = w & 1`, uniform value
//!    `s = (w >> 1) mod 2^u` (`u <= 63`).
//!
//! 3. nested lane maps only (spec/DESIGN.md section 16): then one more little-endian `u64` `x`
//!    per lane. If `x & 1 == 1` the lane is diagonal (its second-pass inner value `b` equals its
//!    inner value `a`); otherwise `b = (x >> 1) mod 2^w`. The lanes of step 2 are unchanged.
//!
//! Every input file is committed to by the seed, so changing any byte of the circuit, lane map,
//! family or spec redraws every lane: a submission cannot choose which lanes are tested.
//!
//! **Fresh-seed audits** (spec/DESIGN.md section 14). Inside [`with_audit`], [`seed`] is instead
//! `SHAKE256("femoco-walk-fiat-shamir-audit-v1" || the same four digests || u64le(round) ||
//! randomness)`, where `round` and `randomness` are a public drand beacon value fixed after the
//! circuits were frozen; steps 1-3 then read that stream exactly as above. Outside the scope
//! (every ordinary run) the seed is the formula above, byte for byte
//! (`tests/seed_stream.rs`). The scope is per thread: `eval_circuit` sets it around
//! `score::evaluate`, which draws every lane and the measurement key on the calling thread.
use sha3::digest::{ExtendableOutput, Update, XofReader};
use sha3::{Shake256, Shake256Reader};
use std::cell::Cell;

pub const DOMAIN: &[u8] = b"femoco-walk-fiat-shamir-v1";
pub const HMR_DOMAIN: &[u8] = b"femoco-walk-hmr-v1";
/// Domain string of the audit seed; distinct from [`DOMAIN`], so no ordinary seed equals an
/// audit seed unless SHAKE256 collides.
pub const AUDIT_DOMAIN: &[u8] = b"femoco-walk-fiat-shamir-audit-v1";
/// Default sample count of a fresh-seed audit, `2^21` (four times the ordinary `2^19`):
/// `f_bound = ln(1e6) / 2^21 = 6.6e-6`, single draw.
pub const AUDIT_SAMPLES: usize = 1 << 21;

/// The drand network whose beacon fixes the audit seed: `quicknet`, as served by its HTTP
/// relays (`/info`, read 2026-09-23). Rounds are unchained (each signature signs only its round
/// number) and one is produced every 3 s.
pub mod quicknet {
    use sha2::{Digest, Sha256};

    pub const CHAIN_HASH: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
    pub const PUBLIC_KEY: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c\
8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a\
6e9c76a4bc09e76eae8991ef5ece45a";
    pub const SCHEME: &str = "bls-unchained-g1-rfc9380";
    pub const GENESIS_TIME: u64 = 1_692_803_367;
    pub const PERIOD: u64 = 3;

    /// Unix time at which `round` (1-based) is produced.
    #[must_use]
    pub fn round_time(round: u64) -> u64 {
        GENESIS_TIME + round.saturating_sub(1) * PERIOD
    }

    /// Whether `randomness = SHA-256(signature)`, drand's definition of a round's randomness.
    /// This ties the recorded value to the recorded signature; it does not check the BLS
    /// signature itself (a drand client must do that).
    #[must_use]
    pub fn randomness_matches(signature: &[u8], randomness: &[u8; 32]) -> bool {
        let h: [u8; 32] = Sha256::digest(signature).into();
        &h == randomness
    }
}

/// A public randomness value fixed after the audited circuits were frozen: a drand quicknet
/// round number and its 32-byte randomness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditBeacon {
    pub round: u64,
    pub randomness: [u8; 32],
}

thread_local! {
    static AUDIT: Cell<Option<AuditBeacon>> = const { Cell::new(None) };
}

/// The audit beacon in force on this thread, if any.
#[must_use]
pub fn active_audit() -> Option<AuditBeacon> {
    AUDIT.with(Cell::get)
}

/// Runs `f` with every seed on this thread drawn from the audit stream of `beacon`, then
/// restores the previous state (also if `f` panics).
pub fn with_audit<R>(beacon: AuditBeacon, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<AuditBeacon>);
    impl Drop for Restore {
        fn drop(&mut self) {
            AUDIT.with(|a| a.set(self.0));
        }
    }
    let _restore = Restore(AUDIT.with(|a| a.replace(Some(beacon))));
    f()
}

/// SHA-256 digests of the four inputs the seed commits to.
#[derive(Clone, Copy, Debug)]
pub struct Digests {
    pub spec: [u8; 32],
    pub lanemap: [u8; 32],
    pub family: [u8; 32],
    pub ops: [u8; 32],
}

/// One sampled lane: control bit and uniform value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    pub c: bool,
    pub s: u64,
}

/// The sampled lanes and the measurement key.
pub struct Sample {
    pub lanes: Vec<Lane>,
    pub hmr_key: [u8; 32],
}

/// The seed stream: the ordinary formula, or the audit stream inside [`with_audit`].
#[must_use]
pub fn seed(d: &Digests) -> Shake256Reader {
    if let Some(b) = active_audit() {
        return audit_seed(d, &b);
    }
    let mut h = Shake256::default();
    h.update(DOMAIN);
    for part in [d.spec, d.lanemap, d.family, d.ops] {
        h.update(&part);
    }
    h.finalize_xof()
}

/// The audit seed stream: `SHAKE256(AUDIT_DOMAIN || spec || lanemap || family || ops ||
/// u64le(round) || randomness)`.
#[must_use]
pub fn audit_seed(d: &Digests, b: &AuditBeacon) -> Shake256Reader {
    let mut h = Shake256::default();
    h.update(AUDIT_DOMAIN);
    for part in [d.spec, d.lanemap, d.family, d.ops] {
        h.update(&part);
    }
    h.update(&b.round.to_le_bytes());
    h.update(&b.randomness);
    h.finalize_xof()
}

/// Draws the measurement key and `k` lanes for a uniform register of `u` bits.
///
/// # Panics
/// If `u > 63`.
#[must_use]
pub fn sample(d: &Digests, u: u32, k: usize) -> Sample {
    draw(d, u, k).0
}

/// A nested sample: the lanes of `sample`, and per lane the uniform value after the `Reflect`
/// (inner bits `lo .. lo + w` replaced by `b`) and whether the lane is diagonal (`b = a`).
pub struct NestedSample {
    pub sample: Sample,
    pub after: Vec<u64>,
    pub diagonal: Vec<bool>,
}

/// `sample`, then one `u64` per lane for the second-pass inner value (step 3 above).
///
/// # Panics
/// If `u > 63` or the inner register does not fit inside it.
#[must_use]
pub fn sample_nested(d: &Digests, u: u32, lo: u32, w: u32, k: usize) -> NestedSample {
    assert!(
        lo + w <= u && w < 64,
        "inner register outside the uniform register"
    );
    let (sample, mut xof) = draw(d, u, k);
    let inner = ((1u64 << w) - 1) << lo;
    let (mut after, mut diagonal) = (Vec::with_capacity(k), Vec::with_capacity(k));
    for lane in &sample.lanes {
        let mut b = [0u8; 8];
        xof.read(&mut b);
        let x = u64::from_le_bytes(b);
        let diag = x & 1 == 1;
        let second = if diag {
            lane.s & inner
        } else {
            ((x >> 1) << lo) & inner
        };
        after.push((lane.s & !inner) | second);
        diagonal.push(diag);
    }
    NestedSample {
        sample,
        after,
        diagonal,
    }
}

fn draw(d: &Digests, u: u32, k: usize) -> (Sample, Shake256Reader) {
    assert!(u <= 63, "uniform register wider than 63 bits");
    let mut xof = seed(d);
    let mut hmr_key = [0u8; 32];
    xof.read(&mut hmr_key);
    let mask = (1u64 << u) - 1;
    let lanes = (0..k)
        .map(|_| {
            let mut b = [0u8; 8];
            xof.read(&mut b);
            let w = u64::from_le_bytes(b);
            Lane {
                c: w & 1 == 1,
                s: (w >> 1) & mask,
            }
        })
        .collect();
    (Sample { lanes, hmr_key }, xof)
}

/// The `Hmr` outcome stream for one batch.
#[must_use]
pub fn hmr_stream(key: &[u8; 32], batch: u64) -> Shake256Reader {
    let mut h = Shake256::default();
    h.update(HMR_DOMAIN);
    h.update(key);
    h.update(&batch.to_le_bytes());
    h.finalize_xof()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_input_byte_changes_the_lanes() {
        let d = Digests {
            spec: [1; 32],
            lanemap: [2; 32],
            family: [3; 32],
            ops: [4; 32],
        };
        let a = sample(&d, 20, 64).lanes;
        let mut d2 = d;
        d2.ops[31] ^= 1;
        assert_ne!(a, sample(&d2, 20, 64).lanes);
        assert_eq!(a, sample(&d, 20, 64).lanes);
        assert!(a.iter().all(|l| l.s < 1 << 20));
        // The nested sample keeps the same lanes and only adds the second-pass values.
        let n = sample_nested(&d, 20, 12, 8, 64);
        assert_eq!(n.sample.lanes, a);
        for ((l, &after), &diag) in a.iter().zip(&n.after).zip(&n.diagonal) {
            assert_eq!(after & 0xfff, l.s & 0xfff, "outer bits unchanged");
            assert!(after < 1 << 20);
            if diag {
                assert_eq!(after, l.s);
            }
        }
        assert!(n.diagonal.iter().any(|&x| x) && n.diagonal.iter().any(|&x| !x));
    }

    #[test]
    fn quicknet_constants() {
        assert_eq!(quicknet::PUBLIC_KEY.len(), 192);
        assert!(
            quicknet::PUBLIC_KEY.starts_with("83cf0f28")
                && quicknet::PUBLIC_KEY.ends_with("f5ece45a")
        );
        assert_eq!(quicknet::round_time(1), quicknet::GENESIS_TIME);
        assert_eq!(quicknet::round_time(11), quicknet::GENESIS_TIME + 30);
        // A published round (drand.cloudflare.com, fetched 2026-09-23).
        let sig = hex::decode("96bd15c0cf87e80cb15f529ab183a02bf41cd46a8d1238a9a2ef72a5de407587cea475d1d4c7bc56fdf1c8c243f83ca0").unwrap();
        let mut r = [0u8; 32];
        hex::decode_to_slice(
            "cf7af396107693b28cd6d13d64de12fb22d9b849747d37ed0ecc545871bd2165",
            &mut r,
        )
        .unwrap();
        assert!(quicknet::randomness_matches(&sig, &r));
        r[0] ^= 1;
        assert!(!quicknet::randomness_matches(&sig, &r));
    }
}
