//! Facts about a circuit the taxonomy checks read. See spec/DESIGN.md section 10.
//! `static_facts` fills everything readable from the op stream; `eval` adds the spec, lane-map
//! and executed-count fields after simulation.
use crate::circuit::{Op, OperationType as K, NONE, SEG_PREPARE, SEG_UNPREPARE};
use crate::sim::Layout;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Segment code for ops before any `Segment` hint.
pub const NO_SEGMENT: u8 = 255;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CircuitFacts {
    pub spec_id: String,
    pub encoding: String,
    pub lane_map_family: String,
    pub u: u32,
    /// Alias index bits and keep bits, when the lane map has them.
    pub k: Option<u32>,
    pub mu: Option<u32>,
    /// Op counts by kind name, overall.
    pub op_counts: BTreeMap<String, u64>,
    /// Op counts by segment code (0 prepare, 1 select, 2 unprepare, 3 inner-begin, 4 inner-end),
    /// then by kind name. Ops outside any segment are under code 255.
    pub segment_op_counts: BTreeMap<u8, BTreeMap<String, u64>>,
    /// Executed counts averaged over sampled lanes.
    pub executed_ccx: f64,
    pub executed_ccz: f64,
    pub executed_hmr: f64,
    pub executed_givens: f64,
    /// Some `Hmr` is followed, within its segment, by a conditioned `CZ`/`Z` on the measured
    /// qubit's former controls (measurement-based uncomputation).
    pub measurement_uncompute: bool,
    pub condition_bits: u64,
    /// Deepest condition nesting: open `PUSH_CONDITION`s plus one for an op's own condition.
    pub max_condition_depth: u32,
    /// Ancilla high-water mark per segment code.
    pub segment_ancilla_peak: BTreeMap<u8, u64>,
    /// The unprepare segment is the op-by-op inverse of the prepare segment.
    pub prepare_unprepare_inverse: bool,
    /// Per segment code, ops that act on a system qubit (in any operand: a system qubit may sit
    /// in any position of a `CZ`/`CCZ`), by kind name.
    pub segment_system_ops: BTreeMap<u8, BTreeMap<String, u64>>,
    /// Op depth and Toffoli (`CCX`/`CCZ`) depth over qubit wires, ignoring conditions.
    pub depth: u64,
    pub toffoli_depth: u64,
    /// Peak live qubits (the score's `Q_peak`, before any phase-gradient register).
    pub q_peak: u64,
    /// The `Segment` hints form one walk step: codes 0, 1, 2 in that order, each once, with 3/4
    /// only inside 1 (paired, 3 before 4); every op that acts on a system qubit lies in 1, 3
    /// or 4; and no op other than register bookkeeping precedes the first hint. Hints are the
    /// submitter's labels: this checks their structure, not that each op belongs where it is.
    /// `evaluate` clears it when inner hints (3/4) appear without a nested lane map: ops inside
    /// them are counted under 3/4, not under SELECT's code 1, so they could hide SELECT's
    /// Toffolis from the `select` signatures.
    pub segments_validated: bool,
    /// The lane map is nested (df-nested-alias-v1) and the harness accepted the nested structure:
    /// two identical inner copies around exactly one `Reflect` on the inner uniform register,
    /// which nothing else touches (spec/DESIGN.md section 16.3). False for every other run.
    pub nested_validated: bool,
    /// Width of the inner uniform register, for nested lane maps.
    pub inner_uniform_bits: Option<u32>,
}

/// Per-kind tallies indexed by the kind's wire code (all codes are below 64).
type Counts = [u64; 64];

fn named(c: &Counts) -> BTreeMap<String, u64> {
    K::ALL
        .iter()
        .filter(|k| c[**k as usize] > 0)
        .map(|k| (k.name().to_string(), c[*k as usize]))
        .collect()
}

/// Every field readable from the op stream alone.
#[must_use]
pub fn static_facts(ops: &[Op], layout: &Layout) -> CircuitFacts {
    let mut f = CircuitFacts::default();
    let mut seg = NO_SEGMENT;
    let mut stack: Vec<u32> = Vec::new();
    let mut cond_bits = BTreeSet::new();
    let mut mu = MeasureScan::default();
    let (mut all, mut by_seg, mut sys): (Counts, BTreeMap<u8, Counts>, BTreeMap<u8, Counts>) =
        ([0; 64], BTreeMap::new(), BTreeMap::new());
    for op in ops {
        if op.kind == K::Segment {
            seg = u8::try_from(op.r_target).unwrap_or(NO_SEGMENT);
            mu.pending.clear();
        }
        let k = op.kind as usize;
        all[k] += 1;
        by_seg.entry(seg).or_insert([0; 64])[k] += 1;
        if acts_on_system(op, layout) {
            sys.entry(seg).or_insert([0; 64])[k] += 1;
        }
        if op.c_condition != NONE {
            cond_bits.insert(op.c_condition);
        }
        let own = u32::from(op.c_condition != NONE && op.kind != K::PushCondition);
        match op.kind {
            K::PushCondition => stack.push(op.c_condition),
            K::PopCondition => {
                stack.pop();
            }
            _ => {}
        }
        f.max_condition_depth = f.max_condition_depth.max(stack.len() as u32 + own);
        f.measurement_uncompute |= mu.step(op, &stack);
    }
    f.op_counts = named(&all);
    f.segment_op_counts = by_seg.iter().map(|(s, c)| (*s, named(c))).collect();
    f.segment_system_ops = sys.iter().map(|(s, c)| (*s, named(c))).collect();
    // A segment with no system ops reads as zero, not as unknown.
    for code in f.segment_op_counts.keys() {
        f.segment_system_ops.entry(*code).or_default();
    }
    f.segments_validated = segments_validated(ops, layout);
    f.condition_bits = cond_bits.len() as u64;
    f.prepare_unprepare_inverse = prepare_unprepare_inverse(ops);
    (f.depth, f.toffoli_depth) = depths(ops);
    f
}

/// Whether `op` names a system qubit in any operand. `CZ`/`CCZ` are symmetric and the harness
/// accepts a system qubit in any of their positions, so the target alone would miss a system
/// `CZ` written with the system qubit as a control. A `SpinSwap`
/// (sos-sa only) acts on the whole system register through its implicit operand.
fn acts_on_system(op: &Op, layout: &Layout) -> bool {
    matches!(op.kind, K::SpinSwap | K::SpinSwapDg) || op.qubits().any(|q| layout.sys(q).is_some())
}

/// See [`CircuitFacts::segments_validated`].
fn segments_validated(ops: &[Op], layout: &Layout) -> bool {
    let bookkeeping = |k: K| {
        matches!(
            k,
            K::Register | K::AppendToRegister | K::DebugPrint | K::Segment
        )
    };
    let (mut seg, mut next_top, mut inner_open) = (NO_SEGMENT, 0u8, false);
    for op in ops {
        if op.kind == K::Segment {
            let code = u8::try_from(op.r_target).unwrap_or(NO_SEGMENT);
            let ok = match code {
                0..=2 => !inner_open && code == next_top,
                3 => seg == 1 && !inner_open,
                4 => inner_open,
                _ => false,
            };
            if !ok {
                return false;
            }
            match code {
                0..=2 => next_top = code + 1,
                3 => inner_open = true,
                _ => inner_open = false,
            }
            // After an inner-end the ops are still inside SELECT.
            seg = if code == 4 { 1 } else { code };
            continue;
        }
        if seg == NO_SEGMENT && !bookkeeping(op.kind) {
            return false;
        }
        if acts_on_system(op, layout) && !matches!(seg, 1 | 3 | 4) {
            return false;
        }
    }
    next_top == 3 && !inner_open
}

/// Tracks, per qubit, the controls of the last `CCX` into it, and the `Hmr`s (by outcome bit)
/// awaiting a conditioned phase fix-up on those controls.
#[derive(Default)]
struct MeasureScan {
    ccx_controls: HashMap<u32, [u32; 2]>,
    pending: HashMap<u32, [u32; 2]>,
}

impl MeasureScan {
    fn step(&mut self, op: &Op, stack: &[u32]) -> bool {
        match op.kind {
            K::CCX => {
                self.ccx_controls
                    .insert(op.q_target, [op.q_control1, op.q_control2]);
            }
            K::Hmr => {
                if let Some(c) = self.ccx_controls.remove(&op.q_target) {
                    self.pending.insert(op.c_target, c);
                }
            }
            K::Z | K::CZ => {
                return stack.iter().chain([&op.c_condition]).any(|bit| {
                    self.pending
                        .get(bit)
                        .is_some_and(|ctrl| op.qubits().all(|q| ctrl.contains(&q)))
                });
            }
            _ => {}
        }
        false
    }
}

/// An op with its full condition set, in a form where equality means "the same gate".
#[derive(PartialEq, Eq)]
struct Canon {
    kind: K,
    target: u32,
    others: BTreeSet<u32>,
    conds: BTreeSet<u32>,
}

fn canon(op: &Op, stack: &[u32], invert: bool) -> Option<Canon> {
    let kind = match (op.kind, invert) {
        (K::S, true) => K::Sdg,
        (K::Sdg, true) => K::S,
        (K::X | K::Z | K::CX | K::CZ | K::Swap | K::CCX | K::CCZ | K::Neg | K::S | K::Sdg, _) => {
            op.kind
        }
        _ => return None,
    };
    let symmetric = matches!(kind, K::CZ | K::CCZ | K::Swap);
    let mut others: BTreeSet<u32> = [op.q_control1, op.q_control2]
        .into_iter()
        .filter(|&q| q != NONE)
        .collect();
    let target = if symmetric {
        others.insert(op.q_target);
        NONE
    } else {
        op.q_target
    };
    let conds = stack
        .iter()
        .copied()
        .chain([op.c_condition])
        .filter(|&b| b != NONE)
        .collect();
    Some(Canon {
        kind,
        target,
        others,
        conds,
    })
}

/// Collects a segment's gates with their condition sets; `None` if it holds a non-invertible op.
fn segment_gates(ops: &[Op], code: u32, invert: bool) -> Option<Vec<Canon>> {
    let (mut seg, mut stack, mut out) = (NO_SEGMENT as u32, Vec::new(), Vec::new());
    for op in ops {
        match op.kind {
            K::Segment => seg = op.r_target,
            K::PushCondition => stack.push(op.c_condition),
            K::PopCondition => {
                stack.pop();
            }
            K::DebugPrint | K::Register | K::AppendToRegister => {}
            _ if seg == code => out.push(canon(op, &stack, invert)?),
            _ => {}
        }
    }
    Some(out)
}

fn prepare_unprepare_inverse(ops: &[Op]) -> bool {
    let (Some(p), Some(mut q)) = (
        segment_gates(ops, SEG_PREPARE, true),
        segment_gates(ops, SEG_UNPREPARE, false),
    ) else {
        return false;
    };
    q.reverse();
    !p.is_empty() && p == q
}

fn depths(ops: &[Op]) -> (u64, u64) {
    let (mut d, mut t) = (Vec::<(u64, u64)>::new(), (0u64, 0u64));
    for op in ops {
        if matches!(
            op.kind,
            K::Segment | K::DebugPrint | K::Register | K::AppendToRegister
        ) {
            continue;
        }
        if op.qubits().next().is_none() {
            continue;
        }
        let (a, b) = op.qubits().fold((0, 0), |(a, b), q| {
            let (x, y) = d.get(q as usize).copied().unwrap_or((0, 0));
            (a.max(x), b.max(y))
        });
        let toff = u64::from(matches!(op.kind, K::CCX | K::CCZ));
        let v = (a + 1, b + toff);
        for q in op.qubits() {
            if d.len() <= q as usize {
                d.resize(q as usize + 1, (0, 0));
            }
            d[q as usize] = v;
        }
        t = (t.0.max(v.0), t.1.max(v.1));
    }
    t
}
