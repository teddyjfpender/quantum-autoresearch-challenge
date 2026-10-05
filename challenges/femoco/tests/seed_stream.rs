//! The ordinary Fiat-Shamir seed stream is unchanged by the fresh-seed audit mode
//! (spec/DESIGN.md section 14), and the audit stream is separate from it.
//!
//! The golden values below were computed independently of this crate, in Python, from the
//! formula in `src/fiat_shamir.rs` (`hashlib.shake_256(b"femoco-walk-fiat-shamir-v1" +
//! bytes([1]*32) + bytes([2]*32) + bytes([3]*32) + bytes([4]*32))`, read as documented), and the
//! test passed against the code on `main` before the audit mode was added.
use femoco_walk::fiat_shamir::{
    active_audit, audit_seed, sample, sample_nested, seed, with_audit, AuditBeacon, Digests,
    AUDIT_DOMAIN, DOMAIN,
};
use sha3::digest::{ExtendableOutput, Update, XofReader};
use sha3::Shake256;

fn digests() -> Digests {
    Digests {
        spec: [1; 32],
        lanemap: [2; 32],
        family: [3; 32],
        ops: [4; 32],
    }
}

const GOLDEN_KEY: &str = "f6dc7af3d2f755750277a85cbea0039c290992371bd12b15ec29d126b4c6c2b1";
const GOLDEN_LANES: [(bool, u64); 8] = [
    (false, 242_141),
    (false, 961_853),
    (false, 992_027),
    (true, 316_661),
    (false, 301_171),
    (false, 80_668),
    (false, 683_673),
    (true, 716_370),
];
/// `sample_nested(d, 20, 12, 8, 8)`: (uniform value after the Reflect, diagonal).
const GOLDEN_NESTED: [(u64, bool); 8] = [
    (545_245, false),
    (961_853, true),
    (992_027, true),
    (316_661, true),
    (125_043, false),
    (969_500, false),
    (683_673, true),
    (822_866, false),
];

fn assert_golden(d: &Digests) {
    let s = sample(d, 20, 8);
    assert_eq!(hex::encode(s.hmr_key), GOLDEN_KEY);
    let got: Vec<(bool, u64)> = s.lanes.iter().map(|l| (l.c, l.s)).collect();
    assert_eq!(got, GOLDEN_LANES);
    let n = sample_nested(d, 20, 12, 8, 8);
    let got: Vec<(u64, bool)> = n.after.iter().copied().zip(n.diagonal).collect();
    assert_eq!(got, GOLDEN_NESTED);
}

#[test]
fn ordinary_seed_stream_is_unchanged() {
    assert_eq!(DOMAIN, b"femoco-walk-fiat-shamir-v1");
    assert!(active_audit().is_none());
    assert_golden(&digests());
}

fn beacon() -> AuditBeacon {
    AuditBeacon {
        round: 32_460_898,
        randomness: [7; 32],
    }
}

#[test]
fn audit_stream_is_the_documented_formula_and_differs() {
    let d = digests();
    // SHAKE256(AUDIT_DOMAIN || spec || lanemap || family || ops || u64le(round) || randomness).
    let mut h = Shake256::default();
    h.update(b"femoco-walk-fiat-shamir-audit-v1");
    for part in [d.spec, d.lanemap, d.family, d.ops] {
        h.update(&part);
    }
    h.update(&32_460_898u64.to_le_bytes());
    h.update(&[7; 32]);
    let mut want = [0u8; 96];
    h.finalize_xof().read(&mut want);
    assert_eq!(AUDIT_DOMAIN, b"femoco-walk-fiat-shamir-audit-v1");
    let mut got = [0u8; 96];
    audit_seed(&d, &beacon()).read(&mut got);
    assert_eq!(got, want);

    // Inside the scope, `seed` and every sampler use the audit stream.
    let (key, lanes) = with_audit(beacon(), || {
        assert_eq!(active_audit(), Some(beacon()));
        let mut inner = [0u8; 96];
        seed(&d).read(&mut inner);
        assert_eq!(inner, want);
        let s = sample(&d, 20, 8);
        (s.hmr_key, s.lanes)
    });
    assert_eq!(key[..], want[..32]);
    assert_ne!(hex::encode(key), GOLDEN_KEY);
    let ordinary: Vec<(bool, u64)> = lanes.iter().map(|l| (l.c, l.s)).collect();
    assert_ne!(ordinary, GOLDEN_LANES);

    // A different round or randomness changes the lanes.
    let other = AuditBeacon {
        round: 32_460_899,
        ..beacon()
    };
    let lanes2 = with_audit(other, || sample(&d, 20, 8).lanes);
    assert_ne!(lanes, lanes2);

    // The scope ends: the ordinary stream is back, byte for byte.
    assert!(active_audit().is_none());
    assert_golden(&d);
}

#[test]
fn audit_scope_is_restored_after_a_panic() {
    let r = std::panic::catch_unwind(|| with_audit(beacon(), || panic!("inside the audit")));
    assert!(r.is_err());
    assert!(active_audit().is_none());
    assert_golden(&digests());
}

#[test]
fn audit_scope_does_not_leak_to_other_threads() {
    with_audit(beacon(), || {
        std::thread::spawn(|| {
            assert!(active_audit().is_none());
            assert_golden(&digests());
        })
        .join()
        .unwrap();
    });
}
