//! Static pass over the op stream: enforces the system-wire rules, balances the condition
//! stack, resolves registers, measures liveness (`Q_peak`) and lowers ops to `SimOp`.
use super::liveness::{track, Liveness};
use super::nested::{check_structure, Inner};
use super::tracker::TrackerFactory;
use crate::circuit::{Op, OperationType as K, NONE};
use std::collections::BTreeMap;

/// Caps so a forged op stream cannot make the simulator allocate without bound.
/// Each qubit or bit costs `8 W` = 64 bytes per batch per thread.
pub const MAX_QUBITS: u32 = 1 << 20;
pub const MAX_BITS: u32 = 1 << 22;
pub const MAX_REGISTERS: u32 = 1 << 20;

/// The harness's fixed qubit layout (spec/DESIGN.md section 4).
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// System register width `2N`.
    pub system: usize,
    /// Uniform register width `u`.
    pub uniform: u32,
}

impl Layout {
    /// First ancilla id: after control (1), system (2N) and uniform (u).
    #[must_use]
    pub fn first_ancilla(&self) -> u32 {
        1 + self.system as u32 + self.uniform
    }
    /// The system index of qubit `q`, if it is a system qubit.
    #[must_use]
    pub fn sys(&self, q: u32) -> Option<u32> {
        (q != NONE && q >= 1 && (q as usize) <= self.system).then(|| q - 1)
    }
}

/// A lowered op. Qubit ids are simulator wire ids; `cond` is a classical bit or `NONE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimOp {
    Neg {
        cond: u32,
    },
    X {
        t: u32,
        cond: u32,
    },
    Cx {
        c: u32,
        t: u32,
        cond: u32,
    },
    Ccx {
        a: u32,
        b: u32,
        t: u32,
        cond: u32,
    },
    Swap {
        a: u32,
        b: u32,
        cond: u32,
    },
    /// Adds `k` (in Z/8) where every listed qubit is 1: Z/CZ/CCZ (`k = 4`), S (2), Sdg (6).
    Phase {
        q: [u32; 3],
        k: u8,
        cond: u32,
    },
    Hmr {
        t: u32,
        bit: u32,
        cond: u32,
    },
    Reset {
        t: u32,
    },
    /// 0 invert, 1 store 0, 2 store 1.
    Bit {
        bit: u32,
        kind: u8,
        cond: u32,
    },
    Push {
        bit: u32,
    },
    Pop,
    /// `X` (or `Z` when `z`) on system qubit `q` where both non-system controls are 1.
    Sys {
        q: u32,
        z: bool,
        ctrl: [u32; 2],
        cond: u32,
    },
    /// `S^k` on system qubit `q` (`k` = 1 or 3).
    SysS {
        q: u32,
        k: u8,
        cond: u32,
    },
    Givens {
        p: u32,
        q: u32,
        reg: u32,
        cond: u32,
    },
    /// Nested DF: `2|+><+| - I` on the inner uniform register. A lane simulates it as "the
    /// inner register holds `a`; make it `b`" (spec/DESIGN.md section 16).
    Reflect,
    /// sos-sa only (spec/SPEC-SA.md section 11): where non-system qubit `c` is 1, the layer
    /// `F = prod_p G_{2p, 2p+1}(pi / 2)` on the system register.
    SpinSwap {
        c: u32,
        cond: u32,
        /// `F^dagger` (`SpinSwapDg`) instead of `F`.
        dagger: bool,
    },
}

/// The lowered stream plus what the static pass measured.
pub struct Compiled {
    pub ops: Vec<SimOp>,
    pub num_qubits: u32,
    pub num_bits: u32,
    /// Register id -> its qubits, little-endian.
    pub registers: Vec<Vec<u32>>,
    /// Peak live qubits (control + system + uniform + live ancillas).
    pub q_peak: u64,
    /// Peak live ancillas per segment code (255 = before any segment hint).
    pub segment_ancilla_peak: BTreeMap<u8, u64>,
    pub uses_givens: bool,
}

fn reject(i: usize, op: &Op, why: &str) -> String {
    format!("op {i} ({}): {why}", op.kind.name())
}

fn free_of_system(l: &Layout, qs: &[u32]) -> bool {
    qs.iter().all(|&q| l.sys(q).is_none())
}

/// Lowers a gate that touches a system wire. Proves: the system register is only ever the
/// target of a Pauli (`X`, `Z`, or `CX`/`CCX`/`CZ`/`CCZ` whose other wires are non-system) or of
/// `S`/`Sdg`, so no non-system wire depends on the system state and the lane's system action is
/// exactly the tracked frame.
fn lower_system(l: &Layout, op: &Op, cond: u32) -> Result<SimOp, &'static str> {
    let (t, c1, c2) = (op.q_target, op.q_control1, op.q_control2);
    let sys_t = l.sys(t);
    let pick = |qs: [u32; 3]| -> Option<(u32, [u32; 2])> {
        let sys: Vec<usize> = (0..3).filter(|&i| l.sys(qs[i]).is_some()).collect();
        let [only] = sys[..] else { return None };
        let others: Vec<u32> = (0..3).filter(|&i| i != only).map(|i| qs[i]).collect();
        l.sys(qs[only]).map(|q| (q, [others[0], others[1]]))
    };
    let rule = "a system qubit may only be the target of X/Z/S/Sdg or of CX/CCX/CZ/CCZ whose \
                other qubits are all non-system";
    match op.kind {
        K::X | K::Z if sys_t.is_some() => Ok(SimOp::Sys {
            q: sys_t.unwrap_or(0),
            z: op.kind == K::Z,
            ctrl: [NONE, NONE],
            cond,
        }),
        K::S | K::Sdg => Ok(SimOp::SysS {
            q: sys_t.ok_or(rule)?,
            k: if op.kind == K::S { 1 } else { 3 },
            cond,
        }),
        K::CX | K::CCX if sys_t.is_some() && free_of_system(l, &[c1, c2]) => Ok(SimOp::Sys {
            q: sys_t.unwrap_or(0),
            z: false,
            ctrl: [c1, c2],
            cond,
        }),
        K::CZ | K::CCZ => pick([t, c1, c2])
            .map(|(q, ctrl)| SimOp::Sys {
                q,
                z: true,
                ctrl,
                cond,
            })
            .ok_or(rule),
        K::Hmr | K::R => Err("a system qubit cannot be measured or reset"),
        _ => Err(rule),
    }
}

fn lower_plain(op: &Op, cond: u32) -> Option<SimOp> {
    let (t, c1, c2) = (op.q_target, op.q_control1, op.q_control2);
    Some(match op.kind {
        K::Neg => SimOp::Neg { cond },
        K::X => SimOp::X { t, cond },
        K::CX => SimOp::Cx { c: c1, t, cond },
        K::CCX => SimOp::Ccx {
            a: c1,
            b: c2,
            t,
            cond,
        },
        K::Swap => SimOp::Swap { a: c1, b: t, cond },
        K::Z | K::CZ | K::CCZ => SimOp::Phase {
            q: [t, c1, c2],
            k: 4,
            cond,
        },
        K::S => SimOp::Phase {
            q: [t, NONE, NONE],
            k: 2,
            cond,
        },
        K::Sdg => SimOp::Phase {
            q: [t, NONE, NONE],
            k: 6,
            cond,
        },
        K::Hmr => SimOp::Hmr {
            t,
            bit: op.c_target,
            cond,
        },
        K::R => SimOp::Reset { t },
        K::BitInvert => SimOp::Bit {
            bit: op.c_target,
            kind: 0,
            cond,
        },
        K::BitStore0 => SimOp::Bit {
            bit: op.c_target,
            kind: 1,
            cond,
        },
        K::BitStore1 => SimOp::Bit {
            bit: op.c_target,
            kind: 2,
            cond,
        },
        K::PushCondition => SimOp::Push {
            bit: op.c_condition,
        },
        K::PopCondition => SimOp::Pop,
        _ => return None,
    })
}

fn check_ranges(i: usize, op: &Op) -> Result<(), String> {
    if op.qubits().any(|q| q >= MAX_QUBITS) {
        return Err(reject(i, op, "qubit id above the simulator cap"));
    }
    if [op.c_target, op.c_condition]
        .iter()
        .any(|&b| b != NONE && b >= MAX_BITS)
    {
        return Err(reject(i, op, "bit id above the simulator cap"));
    }
    if op.kind != K::Segment && op.r_target != NONE && op.r_target >= MAX_REGISTERS {
        return Err(reject(i, op, "register id above the simulator cap"));
    }
    Ok(())
}

/// Runs the static pass.
///
/// # Errors
/// The first op that breaks a rule, with its index and the reason.
pub fn compile(
    ops: &[Op],
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
) -> Result<Compiled, String> {
    compile_nested(ops, l, tracker, None)
}

/// `compile`, plus, when the lane map is nested (`inner` is its inner uniform register),
/// accepting `Reflect` and requiring the structure of `nested::check_structure`. Without
/// `inner` a `Reflect` is rejected.
///
/// # Errors
/// As `compile`, or the first structural rule the nested circuit breaks.
pub fn compile_nested(
    ops: &[Op],
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
    inner: Option<Inner>,
) -> Result<Compiled, String> {
    compile_ext(ops, l, tracker, inner, false)
}

/// `compile_nested` for an `sos-sa` spec: also accepts `SpinSwap` (op kind 32,
/// spec/SPEC-SA.md section 11), which every other spec rejects.
///
/// # Errors
/// As `compile_nested`, or a `SpinSwap` without a Givens tracker or on a system qubit.
pub fn compile_sa(
    ops: &[Op],
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
    inner: Option<Inner>,
) -> Result<Compiled, String> {
    compile_ext(ops, l, tracker, inner, true)
}

fn compile_ext(
    ops: &[Op],
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
    inner: Option<Inner>,
    spin_swap: bool,
) -> Result<Compiled, String> {
    let first = l.first_ancilla();
    let base = u64::from(first);
    let mut lv = Liveness::new(base);
    let mut out = Compiled {
        ops: Vec::with_capacity(ops.len()),
        num_qubits: first,
        num_bits: 0,
        registers: Vec::new(),
        q_peak: base,
        segment_ancilla_peak: BTreeMap::new(),
        uses_givens: false,
    };
    let mut depth = 0usize;
    for (i, op) in ops.iter().enumerate() {
        check_ranges(i, op)?;
        let ext = (inner.is_some(), spin_swap);
        if let Some(sim) = lower_one(i, op, l, tracker, ext, &mut out, &mut depth)? {
            out.ops.push(sim);
        }
        track(op, depth, first, &mut lv, &mut out);
    }
    if depth != 0 {
        return Err(format!("{depth} PUSH_CONDITION(s) never popped"));
    }
    out.q_peak = lv.peak;
    out.segment_ancilla_peak = lv.seg_peak;
    if let Some(inner) = inner {
        check_structure(ops, l, inner, &out.registers)?;
    }
    Ok(out)
}

fn lower_one(
    i: usize,
    op: &Op,
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
    (nested, spin_swap): (bool, bool),
    out: &mut Compiled,
    depth: &mut usize,
) -> Result<Option<SimOp>, String> {
    let cond = op.c_condition;
    match op.kind {
        K::SpinSwap | K::SpinSwapDg if !spin_swap => {
            return Err(reject(
                i,
                op,
                "SpinSwap is defined for sos-sa specs only (spec/SPEC-SA.md section 11)",
            ))
        }
        K::SpinSwap | K::SpinSwapDg if tracker.and_then(TrackerFactory::quarter_turn).is_none() => {
            return Err(reject(i, op, "SpinSwap needs the Givens tracker"))
        }
        K::SpinSwap | K::SpinSwapDg if l.sys(op.q_target).is_some() => {
            return Err(reject(
                i,
                op,
                "SpinSwap's control must be a non-system qubit",
            ))
        }
        K::SpinSwap | K::SpinSwapDg => {
            return Ok(Some(SimOp::SpinSwap {
                c: op.q_target,
                cond,
                dagger: op.kind == K::SpinSwapDg,
            }))
        }
        K::Register | K::AppendToRegister => return register(i, op, l, out).map(|()| None),
        K::DebugPrint | K::Segment => return Ok(None),
        K::Reflect if !nested => {
            return Err(reject(
                i,
                op,
                "Reflect needs a nested lane map (df-nested-alias-v1, spec/DESIGN.md section 16)",
            ))
        }
        K::Reflect if out.registers.get(op.r_target as usize).is_none() => {
            return Err(reject(i, op, "Reflect register is not declared before use"))
        }
        K::Reflect => return Ok(Some(SimOp::Reflect)),
        K::Givens => return givens(i, op, l, tracker, out).map(Some),
        K::PushCondition => *depth += 1,
        K::PopCondition if *depth == 0 => return Err(reject(i, op, "pop on an empty stack")),
        K::PopCondition => *depth -= 1,
        K::Hmr | K::R
            if op.q_target < 1 + l.system as u32 + l.uniform && l.sys(op.q_target).is_none() =>
        {
            return Err(reject(
                i,
                op,
                "the control and uniform registers cannot be measured or reset",
            ))
        }
        _ => {}
    }
    if op.qubits().any(|q| l.sys(q).is_some()) {
        return lower_system(l, op, cond)
            .map(Some)
            .map_err(|w| reject(i, op, w));
    }
    Ok(lower_plain(op, cond))
}

fn register(i: usize, op: &Op, l: &Layout, out: &mut Compiled) -> Result<(), String> {
    let r = op.r_target as usize;
    if out.registers.len() <= r {
        out.registers.resize(r + 1, Vec::new());
    }
    if op.kind == K::AppendToRegister {
        if op.q_target == NONE || l.sys(op.q_target).is_some() {
            return Err(reject(i, op, "registers hold non-system qubits only"));
        }
        if out.registers[r].len() >= 64 {
            return Err(reject(i, op, "registers are at most 64 qubits wide"));
        }
        out.registers[r].push(op.q_target);
    }
    Ok(())
}

fn givens(
    i: usize,
    op: &Op,
    l: &Layout,
    tracker: Option<&dyn TrackerFactory>,
    out: &mut Compiled,
) -> Result<SimOp, String> {
    if tracker.is_none() {
        return Err(reject(
            i,
            op,
            "Givens needs the df tracker (not in this build)",
        ));
    }
    // Any two system modes p < q (df extension, spec/DESIGN.md section 15): the Jordan-Wigner string
    // between them is Clifford, so the charge per Givens does not depend on q - p.
    let (Some(p), Some(q)) = (l.sys(op.q_control1), l.sys(op.q_target)) else {
        return Err(reject(
            i,
            op,
            "Givens acts on system qubits p (control1) < q (target)",
        ));
    };
    if p >= q {
        return Err(reject(
            i,
            op,
            "Givens acts on system qubits p (control1) < q (target)",
        ));
    }
    if out.registers.get(op.r_target as usize).is_none() {
        return Err(reject(
            i,
            op,
            "Givens angle register is not declared before use",
        ));
    }
    out.uses_givens = true;
    Ok(SimOp::Givens {
        p,
        q,
        reg: op.r_target,
        cond: op.c_condition,
    })
}
