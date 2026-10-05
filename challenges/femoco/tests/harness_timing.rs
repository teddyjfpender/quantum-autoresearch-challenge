//! Evaluation timing on a synthetic circuit of realistic sparse size (ignored by default; run
//! with `RAYON_NUM_THREADS=4 cargo test --release --test harness_timing -- --ignored
//! --nocapture`). `FEMOCO_TIMING_OPS` (default 30 000 000) and `FEMOCO_TIMING_SAMPLES`
//! (default `score::DEFAULT_SAMPLES`) and `FEMOCO_TIMING_POOL` (data qubits, default 300)
//! override the size.
//!
//! The circuit acts on a 108-qubit system (Reiher's active space) with a 20-bit uniform
//! register and a data pool (300 qubits by default). Each block computes an AND of two uniform bits, fans it
//! out into 30 data qubits (QROM-like) and back, touches the system four times (cancelling),
//! and uncomputes the AND by measurement. The walk it encodes is the identity (a one-term spec),
//! so it passes validation and the timing covers the full per-lane checks.
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::lanemap::alias::AliasMap;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{FamilyOut, Inputs, DEFAULT_SAMPLES};
use femoco_walk::spec::{EncodingSpec, Exact, Monomial, SystemOp};
use femoco_walk::taxonomy::{self, Family, Taxonomy};
use num_bigint::BigInt;
use std::time::Instant;

struct IdSpec;
impl EncodingSpec for IdSpec {
    fn id(&self) -> &str {
        "timing-identity"
    }
    fn encoding(&self) -> &str {
        "test"
    }
    fn spatial_orbitals(&self) -> usize {
        54
    }
    fn lambda(&self) -> Exact {
        Exact::from_int(1)
    }
    fn identity(&self) -> Exact {
        Exact::zero()
    }
    fn payload_sha256(&self) -> [u8; 32] {
        [7; 32]
    }
    fn flat_terms(&self) -> Option<Vec<(Exact, SystemOp)>> {
        Some(vec![(
            Exact::from_int(1),
            SystemOp::Monomial(Monomial {
                phase: 0,
                majoranas: vec![],
            }),
        )])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn circuit(target_ops: usize, pool_size: usize) -> Vec<Op> {
    let mut b = Builder::new(108);
    b.declare_uniform(20);
    let pool = b.alloc_n(pool_size);
    let mut blk = 0usize;
    while b.ops().len() < target_ops {
        let (ui, uj) = (
            b.uniform((blk % 20) as u32),
            b.uniform(((blk * 7 + 3) % 20) as u32),
        );
        let (ui, uj) = if ui == uj {
            (ui, b.uniform(((blk + 1) % 20) as u32))
        } else {
            (ui, uj)
        };
        let a = b.alloc();
        b.ccx(ui, uj, a);
        for pass in 0..2 {
            for k in 0..30 {
                let _ = pass;
                b.cx(a, pool[(blk * 31 + k * 11) % pool_size]);
            }
        }
        let (s1, s2) = (b.system(blk % 108), b.system((blk * 5 + 1) % 108));
        b.cz(a, s1);
        b.cx(a, s2);
        b.cx(a, s2);
        b.cz(a, s1);
        let bit = b.hmr(a);
        b.cz_if(ui, uj, bit);
        blk += 1;
    }
    pool.into_iter().for_each(|q| b.free(q));
    b.finish()
}

#[test]
#[ignore = "timing run; see the module docs"]
fn timing_sparse_sized() {
    let (n_ops, k) = (
        env("FEMOCO_TIMING_OPS", 30_000_000),
        env("FEMOCO_TIMING_SAMPLES", DEFAULT_SAMPLES),
    );
    let t = Instant::now();
    let ops = circuit(n_ops, env("FEMOCO_TIMING_POOL", 300));
    let dir = std::env::temp_dir().join(format!("femoco-timing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(&ops, &path).unwrap();
    println!(
        "built + wrote {} ops in {:.1} s",
        ops.len(),
        t.elapsed().as_secs_f64()
    );
    drop(ops);
    let t = Instant::now();
    let file = read_ops(&path).unwrap();
    let read_s = t.elapsed().as_secs_f64();
    std::fs::remove_dir_all(&dir).ok();
    let map = AliasMap::new(0, 20, Exact::dyadic(BigInt::from(1), 0), vec![0], vec![0]).unwrap();
    let fam = FamilyOut {
        family: Family {
            taxonomy_version: "1.0.0".into(),
            name: "timing".into(),
            parent: None,
            axes: Default::default(),
        },
        spec: "timing-identity".into(),
    };
    let fam = serde_json::to_vec(&fam).unwrap();
    let check = |f: &Family, facts: &femoco_walk::facts::CircuitFacts| {
        taxonomy::check(
            &Taxonomy {
                raw: serde_json::json!({}),
            },
            f,
            facts,
        )
    };
    let ev = evaluate(&Inputs {
        spec: &IdSpec,
        lanemap: &map.to_bytes(),
        family: &fam,
        ops: &file,
        samples: k,
        tracker: None,
        check: &check,
    })
    .unwrap();
    println!(
        "threads {} | ops {} | samples {k} | read {read_s:.1} s | evaluate {:.1} s | toffoli {:.0} | qubits {}",
        rayon::current_num_threads(),
        file.ops.len(),
        ev.seconds,
        ev.toffoli,
        ev.qubits
    );
}
