//! Byte-identity of every pinned spectrum-amplification circuit: each one, rebuilt from the
//! architecture's `Params` (what `build_circuit` compiles from the circuit's `FEMOCO_*` knobs),
//! writes exactly the `ops.bin`, `lanemap.bin` and `family.out.json` whose SHA-256 the measured
//! run recorded. A change to `src/walk/sa_low` that moves any measured circuit fails here; the
//! measured counts then no longer describe the code.
//!
//! The knobs of each row are in the `circuit!` lines of tests/sa_circuits/list.rs and in
//! `src/walk/sa_low/README.md` ("Measured circuits and their digests"). Build this test with no
//! `FEMOCO_SA_*` variable set: `Params::for_spec` reads them at compile time.
//!
//! `FEMOCO_PIN_EXPORT_DIR=<dir>` also writes each circuit's files and a `pin.json` to
//! `<dir>/<pin name>/` (see `build`).
#![cfg(feature = "walk")]
use femoco_walk::circuit::{write_ops, Builder};
use femoco_walk::score::FamilyOut;
use femoco_walk::spec::sa::SaSpec;
use femoco_walk::taxonomy::Family;
use femoco_walk::walk::sa_low::{self, Params, Tweaks};
use sha2::{Digest, Sha256};
use std::path::Path;

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `(ops count, ops sha256, lanemap sha256, family sha256)` of the circuit `build_circuit` would
/// write for `id` with these parameters and family. With `FEMOCO_PIN_EXPORT_DIR` set, the three
/// files are also written to `$FEMOCO_PIN_EXPORT_DIR/<name>/` under `build_circuit`'s names
/// (`ops.bin`, `lanemap.bin`, `family.out.json`; `eval_circuit --root` takes the directory once
/// `specs/` and `taxonomy/` are reachable there), with a `pin.json` describing the circuit.
/// `ctor` is the source text of the parameter constructor.
fn build(
    id: &str,
    name: &str,
    family: Family,
    ctor: &str,
    p: impl Fn(&SaSpec) -> Params,
) -> [String; 4] {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let spec = femoco_walk::spec::load(root, id).unwrap();
    let sa: &SaSpec = spec.as_any().downcast_ref().unwrap();
    let mut b = Builder::new(spec.system_qubits());
    let params = p(sa);
    let lm = sa_low::build_with(sa, &mut b, params);
    let ops = b.finish();
    let path = std::env::temp_dir().join(format!(
        "femoco-sa-digest-{name}-{}.bin",
        std::process::id()
    ));
    write_ops(&ops, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let family_name = family.name.clone();
    let fam = serde_json::to_vec_pretty(&FamilyOut {
        family,
        spec: id.to_string(),
    })
    .unwrap();
    let lm_bytes = lm.to_bytes();
    let got = [
        ops.len().to_string(),
        sha(&bytes),
        sha(&lm_bytes),
        sha(&fam),
    ];
    if let Some(dir) = std::env::var_os("FEMOCO_PIN_EXPORT_DIR") {
        let dir = Path::new(&dir).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ops.bin"), &bytes).unwrap();
        std::fs::write(dir.join("lanemap.bin"), &lm_bytes).unwrap();
        std::fs::write(dir.join("family.out.json"), &fam).unwrap();
        // The tweak string is the constructor's first string literal, when it has one.
        let tweaks = ctor.split('"').nth(1);
        let pin = serde_json::json!({
            "pin": name,
            "spec": id,
            "ops_count": ops.len(),
            "ops_sha256": got[1],
            "lanemap_sha256": got[2],
            "family_sha256": got[3],
            "family_name": family_name,
            "constructor": ctor,
            "tweaks": tweaks,
            "onehot": params.tw.onehot,
            "hot_groups": params.tw.hot_groups,
            "rank_hold": params.tw.rank_hold,
            "pareto": params.pareto,
            "chunks": params.chunks,
            "keep_bits": [params.outer.1, params.inner.1],
            "inner_a": params.inner_a,
            "outer_a": params.outer_a,
        });
        std::fs::write(
            dir.join("pin.json"),
            serde_json::to_vec_pretty(&pin).unwrap(),
        )
        .unwrap();
    }
    got
}

fn check(
    id: &str,
    name: &str,
    family: Family,
    ctor: &str,
    p: impl Fn(&SaSpec) -> Params,
    want: [&str; 4],
) {
    let got = build(id, name, family, ctor, p);
    println!("{name}\t{}\t{}\t{}\t{}", got[0], got[1], got[2], got[3]);
    assert_eq!(
        got,
        want.map(str::to_string),
        "{name}: the measured circuit changed"
    );
}

macro_rules! circuit {
    ($test:ident, $spec:literal, $family:expr, $params:expr, $want:expr) => {
        #[test]
        fn $test() {
            check(
                $spec,
                stringify!($test),
                $family,
                stringify!($params),
                $params,
                $want,
            );
        }
    };
}

// The circuits (shared with tests/equiv_pinned.rs).
include!("sa_circuits/list.rs");
