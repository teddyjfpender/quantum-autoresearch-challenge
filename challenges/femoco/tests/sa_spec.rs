//! The pinned sos-sa specs (spec/SPEC-SA.md): they load, their exact Lambda reproduces Low et al.
//! 2025's printed Table V value, a tampered payload is refused, and (release, ignored) the full lane
//! maps meet the 0.1 mHa rounding rule.
use femoco_walk::lanemap::sa_nested::{self, SaNestedMap};
use femoco_walk::lanemap::{self, LaneMap};
use femoco_walk::spec::sa::SaSpec;
use femoco_walk::spec::{self, EncodingSpec, Exact};
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn load(id: &str) -> Box<dyn EncodingSpec> {
    spec::load(root(), id).unwrap()
}

fn sa(s: &dyn EncodingSpec) -> &SaSpec {
    s.as_any().downcast_ref::<SaSpec>().unwrap()
}

#[test]
fn pinned_specs_load_with_the_published_lambda() {
    for (id, printed, n, rbc, beta) in [
        ("reiher-sa-v1", 58.3440, 54, (10, 27, 27), 16),
        ("li-sa-v1", 179.7296, 76, (15, 57, 19), 15),
    ] {
        let s = load(id);
        assert_eq!(s.encoding(), "sos-sa");
        let d = sa(s.as_ref());
        assert_eq!((d.n, (d.r, d.b, d.c), d.beta), (n, rbc, beta));
        let lam = s.lambda().to_f64();
        // Low et al. 2025 Table V prints Lambda to 4 decimals.
        assert!(
            (lam - printed).abs() < 5e-5,
            "{id}: Lambda {lam} vs printed {printed}"
        );
        // lambda_flat = 2 Lambda - (identity - E_SOS), and H_spec - E_SOS is PSD with E_SOS below identity.
        let two = Exact::from_int(2);
        assert_eq!(
            d.lambda_flat,
            two.mul(&d.lambda).sub(&d.identity.sub(&d.e_sos))
        );
        assert!(d.e_sos < d.identity);
        assert!(d.published_lambda_eff > 20.0);
        let g = femoco_walk::sim::givens_tracker(s.as_ref()).expect("sos-sa has a Givens tracker");
        assert_eq!(g.phase_gradient_qubits(), u64::from(beta));
    }
}

#[test]
fn a_tampered_payload_is_refused() {
    let dir = root().join("specs/reiher-sa-v1");
    let mut bytes = std::fs::read(dir.join("sa.bin")).unwrap();
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("spec.json")).unwrap()).unwrap();
    // One bit of one square weight.
    let at = bytes.len() - 4 * 270 * 53 - 8;
    bytes[at] ^= 1;
    let tmp = std::env::temp_dir().join(format!("femoco-sa-tamper-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("sa.bin"), &bytes).unwrap();
    let e = femoco_walk::spec::sa::load(&tmp, &meta)
        .err()
        .expect("refused");
    std::fs::remove_dir_all(&tmp).ok();
    assert!(e.contains("sha256"), "{e}");
    // The parser itself recomputes the exact values, which then differ from spec.json.
    let p = femoco_walk::spec::sa::parse_payload("x", &bytes).unwrap();
    assert_ne!(
        p.lambda.to_string(),
        meta["lambda"]["exact"].as_str().unwrap()
    );
}

fn map_error(id: &str, outer: (u32, u32), inner: (u32, u32)) -> (SaNestedMap, f64) {
    let s = load(id);
    let map = sa_nested::build(sa(s.as_ref()), outer, inner).unwrap();
    let back = lanemap::parse(&map.to_bytes(), s.as_ref()).unwrap();
    assert_eq!(back.uniform_bits(), map.uniform_bits());
    let err = back.rounding_error(s.as_ref()).unwrap().to_f64();
    (map, err)
}

#[test]
#[ignore = "release: full pinned lane maps"]
fn full_lane_maps_meet_the_rounding_rule() {
    for (id, splits) in [
        (
            "reiher-sa-v1",
            vec![
                ((9, 22), (5, 25)),
                ((9, 23), (5, 24)),
                ((9, 21), (5, 26)),
                ((9, 24), (5, 23)),
            ],
        ),
        (
            "li-sa-v1",
            vec![
                ((9, 23), (6, 23)),
                ((9, 24), (6, 22)),
                ((9, 22), (6, 24)),
                ((9, 25), (6, 21)),
            ],
        ),
    ] {
        for (o, i) in splits {
            let t = std::time::Instant::now();
            let (map, err) = map_error(id, o, i);
            println!(
                "{id} outer {o:?} inner {i:?} u = {} rounding error {err:.4e} Ha ({:.2} s)",
                map.uniform_bits(),
                t.elapsed().as_secs_f64()
            );
        }
    }
}

#[test]
#[ignore = "release: exploration of the smallest widths under 0.1 mHa"]
fn smallest_widths() {
    for (id, k_o, k_i) in [("reiher-sa-v1", 9, 5), ("li-sa-v1", 9, 6)] {
        for total in (14u32..=46).step_by(2) {
            let mut best: Option<(f64, u32)> = None;
            for mu_o in (total.saturating_sub(31)).max(4)..(total - 3).min(32) {
                let mu_i = total - mu_o;
                let (_, err) = map_error(id, (k_o, mu_o), (k_i, mu_i));
                if best.is_none_or(|(e, _)| err < e) {
                    best = Some((err, mu_o));
                }
            }
            let (err, mu_o) = best.unwrap();
            println!(
                "{id} mu_o + mu_i = {total}: best mu_o {mu_o} mu_i {} error {err:.3e} {}",
                total - mu_o,
                if err <= 1e-4 { "PASS" } else { "" }
            );
            if err <= 1e-4 {
                break;
            }
        }
    }
}

#[test]
#[ignore = "release: the authors' 9 keep bits per level"]
fn authors_keep_bits() {
    for (id, k_i) in [("reiher-sa-v1", 5), ("li-sa-v1", 6)] {
        let (map, err) = map_error(id, (9, 9), (k_i, 9));
        println!(
            "{id} (9, 9) ({k_i}, 9) u = {} rounding error {err:.4e} Ha",
            map.uniform_bits()
        );
    }
}
