//! Shared nested-DF fixtures (spec/DESIGN.md section 16): a 3-orbital nested df spec whose
//! weights are dyadic (so exact lane maps exist in both one-body layouts), exact lane maps, and
//! a naive hand-built nested circuit with mutation hooks.
#![allow(dead_code)]
use femoco_walk::circuit::{
    read_ops, write_ops, Builder, Op, OpsFile, Qubit, Reg, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE,
};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::lanemap::df_nested::{DfNestedMap, OneBody, OuterItem, Table};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{Evaluation, FamilyOut, Inputs};
use femoco_walk::sim::givens_tracker;
use femoco_walk::spec::df::{parse_payload, DfSpec};
use femoco_walk::spec::{Exact, Network};
use femoco_walk::taxonomy::{self, AxisVerdict, Family, Taxonomy};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SPATIAL: usize = 3;
pub const BETA: u32 = 8;

/// Deterministic angles in `0..2^BETA`.
pub fn angles(seed: u64, count: usize) -> Vec<u32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << BETA)) as u32
        })
        .collect()
}

/// A `FEMODFS1` payload (the DF payload format; no DF spec ships here).
pub fn payload(ecore: f64, t: &[f64], t_ang: &[u32], leaves: &[(Vec<f64>, Vec<u32>)]) -> Vec<u8> {
    let mut out = b"FEMODFS1".to_vec();
    for v in [1u32, SPATIAL as u32, BETA, leaves.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&ecore.to_le_bytes());
    t.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    t_ang
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    for (e, ang) in leaves {
        out.extend_from_slice(&(e.len() as u32).to_le_bytes());
        e.iter()
            .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        ang.iter()
            .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    }
    out
}

/// t = (4, -2, 2): lambda_T = 8, one-body weights |t_k|/2 = (2, 2, 1, 1, 1, 1) per (k, s).
/// Leaves e = (2, -1, 1) and (3, -1): S = 4 each, so S^2/4 = 4 each and lambda = 16. Inner
/// weights |e|/(2S) are (2,2,1,1,1,1)/8 and (3,3,1,1)/8: every weight is dyadic.
pub fn flat_spec() -> DfSpec {
    let r = SPATIAL - 1;
    let leaves = vec![
        (vec![2.0, -1.0, 1.0], angles(21, 3 * r)),
        (vec![3.0, -1.0], angles(22, 2 * r)),
    ];
    let bytes = payload(0.25, &[4.0, -2.0, 2.0], &angles(20, SPATIAL * r), &leaves);
    parse_payload("test-df-nested-v1", &bytes).unwrap()
}

pub fn spec() -> DfSpec {
    flat_spec().into_nested()
}

/// Exact tables: direct `k_o = 3, mu_o = 1` (16 outer lanes: 2,2,1,1,1,1 one-body, 4, 4 leaves);
/// folded `k_o = 2, mu_o = 2` (8, 4, 4). Inner `k_i = 3, mu_i = 0` (8 lanes).
pub fn exact_map(spec: &DfSpec, one_body: OneBody) -> DfNestedMap {
    let (outer, mut inner) = match one_body {
        OneBody::Direct => (
            Table::from_counts(3, 1, &[2, 2, 1, 1, 1, 1, 4, 4]).unwrap(),
            vec![],
        ),
        OneBody::Folded => (
            Table::from_counts(2, 2, &[8, 4, 4]).unwrap(),
            vec![Table::from_counts(3, 0, &[2, 2, 1, 1, 1, 1]).unwrap()],
        ),
    };
    inner.push(Table::from_counts(3, 0, &[2, 2, 1, 1, 1, 1]).unwrap());
    inner.push(Table::from_counts(3, 0, &[3, 3, 1, 1]).unwrap());
    DfNestedMap::new(one_body, Exact::from_int(16), outer, inner, spec).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    None,
    /// Leaf `l`'s inner term `j` gets an extra -1 (in both copies: the copies stay identical).
    FlipInnerSign {
        l: usize,
        j: usize,
    },
    /// The inner copy leaves inner uniform bit 0 flipped.
    InnerNotRestored,
    /// The folded one-body item is applied in both passes (no pass flag).
    FoldedBothPasses,
}

/// A `(k, mu)` ladder: ANDs `ctrl` with "`bits` hold `v`", returns the indicator.
pub struct Ladder {
    pub sel: Qubit,
    ands: Vec<(Qubit, Qubit, Qubit)>,
    flips: Vec<Qubit>,
}

pub fn ladder_up(b: &mut Builder, ctrl: Qubit, bits: &[Qubit], v: u64) -> Ladder {
    let flips: Vec<Qubit> = bits
        .iter()
        .enumerate()
        .filter(|(i, _)| v >> i & 1 == 0)
        .map(|(_, &q)| q)
        .collect();
    flips.iter().for_each(|&q| b.x(q));
    let mut ands = Vec::new();
    let mut prev = ctrl;
    for &q in bits {
        let a = b.alloc();
        b.ccx(prev, q, a);
        ands.push((prev, q, a));
        prev = a;
    }
    Ladder {
        sel: prev,
        ands,
        flips,
    }
}

pub fn ladder_down(b: &mut Builder, l: Ladder) {
    for &(c1, c2, a) in l.ands.iter().rev() {
        b.ccx(c1, c2, a);
        b.free(a);
    }
    l.flips.iter().for_each(|&q| b.x(q));
}

/// `sign * V Z_spin V^dagger`, every part controlled on `sel` (angles loaded only where `sel`).
fn rotated_z(
    b: &mut Builder,
    sel: Qubit,
    reg: Reg,
    bits: &[Qubit],
    net: &Network,
    spin: u8,
    negative: bool,
) {
    let mode = |p: u16| 2 * usize::from(p) + usize::from(spin);
    let unit = 1u32 << BETA;
    let givens = |b: &mut Builder, p: usize, q: usize, a: u32| {
        let set: Vec<Qubit> = bits
            .iter()
            .enumerate()
            .filter(|(j, _)| a >> j & 1 == 1)
            .map(|(_, &x)| x)
            .collect();
        set.iter().for_each(|&x| b.cx(sel, x));
        b.givens_modes(p, q, reg);
        set.iter().for_each(|&x| b.cx(sel, x));
    };
    for &(p, q, a) in net.rotations.iter().rev() {
        givens(b, mode(p), mode(q), (unit - a) % unit);
    }
    let z = b.system(usize::from(spin));
    b.cz(sel, z);
    for &(p, q, a) in &net.rotations {
        givens(b, mode(p), mode(q), a);
    }
    if negative {
        b.z(sel);
    }
}

/// The naive nested circuit for `map` (the nested DF form; no DF spec ships here): an outer unary loop sets a
/// flag per leaf (and applies direct one-body terms, or sets the folded flag), the inner copy is
/// a unary loop over each flagged table, and `nested_inner` emits it twice around the Reflect.
/// In the folded layout the copy toggles a pass qubit so the one-body item acts in pass 1 only.
pub fn build(spec: &DfSpec, map: &DfNestedMap, m: Mutation) -> Vec<Op> {
    let mut b = Builder::new(2 * SPATIAL);
    b.declare_uniform(map.uniform_bits());
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    let bits = b.alloc_n(BETA as usize);
    let reg = b.register(&bits);
    let inner_reg = b.inner_register(u_o, w);
    let leaves = spec.leaves.len();
    let flags = b.alloc_n(leaves);
    let folded = map.one_body == OneBody::Folded;
    let (flag_t, pass) = (b.alloc(), b.alloc());
    let outer_bits: Vec<Qubit> = (0..u_o).map(|i| b.uniform(i)).collect();
    let inner_bits: Vec<Qubit> = (u_o..u_o + w).map(|i| b.uniform(i)).collect();
    b.segment(SEG_PREPARE);
    b.segment(SEG_SELECT);
    let outer_pass = |b: &mut Builder, apply_one_body: bool| {
        for s in 0..1u64 << u_o {
            let item = map.decode_outer(s);
            let control = b.control();
            let l = ladder_up(b, control, &outer_bits, s);
            match item {
                OuterItem::OneBody { k, spin } if apply_one_body => {
                    rotated_z(b, l.sel, reg, &bits, &spec.t_nets[k], spin, spec.t[k] > 0.0);
                }
                OuterItem::OneBody { .. } => {}
                OuterItem::Folded => b.cx(l.sel, flag_t),
                OuterItem::Leaf(leaf) => b.cx(l.sel, flags[leaf]),
            }
            ladder_down(b, l);
        }
    };
    outer_pass(&mut b, true);
    b.nested_inner(inner_reg, |b| {
        for (leaf, &flag) in flags.iter().enumerate() {
            let table = map.table_of(OuterItem::Leaf(leaf)).unwrap();
            for a in 0..1u64 << w {
                let j = map.inner_item(table, a);
                let (k, spin) = (j >> 1, (j & 1) as u8);
                let neg =
                    (spec.leaves[leaf].e[k] < 0.0) != (m == Mutation::FlipInnerSign { l: leaf, j });
                let l = ladder_up(b, flag, &inner_bits, a);
                rotated_z(b, l.sel, reg, &bits, &spec.leaves[leaf].nets[k], spin, neg);
                ladder_down(b, l);
            }
        }
        if folded {
            // g = flag_T and not pass: the one-body item acts in the first pass only.
            let g = b.alloc();
            let gate = |b: &mut Builder| {
                if m == Mutation::FoldedBothPasses {
                    b.cx(flag_t, g);
                } else {
                    b.x(pass);
                    b.ccx(flag_t, pass, g);
                    b.x(pass);
                }
            };
            gate(b);
            for a in 0..1u64 << w {
                let j = map.inner_item(0, a);
                let (k, spin) = (j >> 1, (j & 1) as u8);
                let l = ladder_up(b, g, &inner_bits, a);
                rotated_z(b, l.sel, reg, &bits, &spec.t_nets[k], spin, spec.t[k] > 0.0);
                ladder_down(b, l);
            }
            gate(b);
            b.free(g);
            if m != Mutation::FoldedBothPasses {
                b.cx(flag_t, pass);
            }
        }
        if m == Mutation::InnerNotRestored {
            b.x(inner_bits[0]);
        }
    });
    outer_pass(&mut b, false);
    b.segment(SEG_UNPREPARE);
    for q in flags.into_iter().chain([flag_t, pass]).chain(bits) {
        b.free(q);
    }
    b.finish()
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// Round-trips `ops` through `ops.bin`.
pub fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-nested-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

pub const NESTED_AXES: [(&str, &str); 7] = [
    ("encoding", "df"),
    ("lane_map", "df-nested-alias-v1"),
    ("lookup", "unary-iteration"),
    ("select", "givens-nested"),
    ("uncompute", "unitary"),
    ("reuse", "serial"),
    ("rotation", "phase-gradient-givens"),
];

pub fn family(spec: &str, axes: &[(&str, &str)]) -> Vec<u8> {
    let axes: BTreeMap<String, String> = axes
        .iter()
        .map(|(a, v)| (a.to_string(), v.to_string()))
        .collect();
    let fam = FamilyOut {
        family: Family {
            taxonomy_version: "1.1.0".into(),
            name: "test-df-nested-naive".into(),
            parent: None,
            axes,
        },
        spec: spec.to_string(),
    };
    serde_json::to_vec(&fam).unwrap()
}

pub fn shipped_taxonomy() -> Taxonomy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    taxonomy::load_taxonomy(&path).unwrap()
}

/// Evaluates through `evaluate` with the shipped taxonomy, except the `uncompute` axis: the
/// naive circuit frees ancillas with `R`, which neither `unitary` (no `R`) nor
/// `measurement-based` (some `Hmr`) admits, and that axis is not what these tests are about.
pub fn eval_ops(
    spec: &DfSpec,
    lanemap: &[u8],
    ops: &[Op],
    axes: &[(&str, &str)],
    samples: usize,
) -> Result<Evaluation, String> {
    let tax = shipped_taxonomy();
    let check = |f: &Family, facts: &femoco_walk::facts::CircuitFacts| -> Vec<AxisVerdict> {
        taxonomy::check(&tax, f, facts)
            .into_iter()
            .filter(|v| v.axis != "uncompute")
            .collect()
    };
    let file = ops_file(ops);
    let fam = family(&spec.id, axes);
    evaluate(&Inputs {
        spec,
        lanemap,
        family: &fam,
        ops: &file,
        samples,
        tracker: givens_tracker(spec),
        check: &check,
    })
}

pub fn eval_mutant(one_body: OneBody, m: Mutation, samples: usize) -> Result<Evaluation, String> {
    let s = spec();
    let map = exact_map(&s, one_body);
    eval_ops(
        &s,
        &map.to_bytes(),
        &build(&s, &map, m),
        &NESTED_AXES,
        samples,
    )
}
