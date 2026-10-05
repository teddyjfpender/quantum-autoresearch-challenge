//! The op stream: kinds, operands, per-op shape validation, `ops.bin` I/O and the `Builder`
//! contestant code emits through. See spec/DESIGN.md sections 4 and 5.
//!
//! Kinds 0-17 keep ecdsa.fail's numbering and meaning. Additions: `S` (20), `Sdg` (21),
//! `Givens` (30, DF only), `Reflect` (31, nested DF only: spec/DESIGN.md section 16.1),
//! `SpinSwap` / `SpinSwapDg` (32 / 33, sos-sa specs only: spec/SPEC-SA.md section 11) and `Segment` (40, a hint).
mod builder;
mod io;

pub use builder::{Bit, Builder, Qubit, Reg};
pub use io::{read_ops, write_ops, OpsFile, MAGIC, MAX_OPS, OP_BYTES};

/// Operand sentinel: "no qubit / bit / register" (`u64::MAX` on disk).
pub const NONE: u32 = u32::MAX;

/// Segment codes carried by `Segment` in `r_target`.
pub const SEG_PREPARE: u32 = 0;
pub const SEG_SELECT: u32 = 1;
pub const SEG_UNPREPARE: u32 = 2;
pub const SEG_INNER_BEGIN: u32 = 3;
pub const SEG_INNER_END: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum OperationType {
    Neg = 0,
    Register = 1,
    AppendToRegister = 2,
    BitInvert = 3,
    BitStore0 = 4,
    BitStore1 = 5,
    X = 6,
    Z = 7,
    CX = 8,
    CZ = 9,
    Swap = 10,
    R = 11,
    Hmr = 12,
    CCX = 13,
    CCZ = 14,
    PushCondition = 15,
    PopCondition = 16,
    DebugPrint = 17,
    S = 20,
    Sdg = 21,
    Givens = 30,
    Reflect = 31,
    /// Controlled swap of the two spin sectors (sos-sa specs only, spec/SPEC-SA.md section 11).
    SpinSwap = 32,
    /// Its inverse, `F^dagger` (sos-sa specs only, spec/SPEC-SA.md section 11).
    SpinSwapDg = 33,
    Segment = 40,
}

impl OperationType {
    /// Every kind, for decoding and reporting.
    pub const ALL: [Self; 25] = [
        Self::Neg,
        Self::Register,
        Self::AppendToRegister,
        Self::BitInvert,
        Self::BitStore0,
        Self::BitStore1,
        Self::X,
        Self::Z,
        Self::CX,
        Self::CZ,
        Self::Swap,
        Self::R,
        Self::Hmr,
        Self::CCX,
        Self::CCZ,
        Self::PushCondition,
        Self::PopCondition,
        Self::DebugPrint,
        Self::S,
        Self::Sdg,
        Self::Givens,
        Self::Reflect,
        Self::SpinSwap,
        Self::SpinSwapDg,
        Self::Segment,
    ];

    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|k| *k as u32 == v)
    }

    /// Upper-case name as in ecdsa.fail's text format (`CCX`, `PUSH_CONDITION`, ...).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Neg => "NEG",
            Self::Register => "REGISTER",
            Self::AppendToRegister => "APPEND_TO_REGISTER",
            Self::BitInvert => "BIT_INVERT",
            Self::BitStore0 => "BIT_STORE0",
            Self::BitStore1 => "BIT_STORE1",
            Self::X => "X",
            Self::Z => "Z",
            Self::CX => "CX",
            Self::CZ => "CZ",
            Self::Swap => "SWAP",
            Self::R => "R",
            Self::Hmr => "HMR",
            Self::CCX => "CCX",
            Self::CCZ => "CCZ",
            Self::PushCondition => "PUSH_CONDITION",
            Self::PopCondition => "POP_CONDITION",
            Self::DebugPrint => "DEBUG_PRINT",
            Self::S => "S",
            Self::Sdg => "SDG",
            Self::Givens => "GIVENS",
            Self::Reflect => "REFLECT",
            Self::SpinSwap => "SPIN_SWAP",
            Self::SpinSwapDg => "SPIN_SWAP_DG",
            Self::Segment => "SEGMENT",
        }
    }

    /// Operand shape `[q_target, q_control1, q_control2, c_target, c_condition, r_target]`.
    fn shape(self) -> [Need; 6] {
        use Need::{Allowed as A, Banned as B, Required as R};
        match self {
            Self::DebugPrint => [A, A, A, A, B, A],
            Self::Register => [B, B, B, B, B, R],
            Self::AppendToRegister => [A, B, B, A, B, R],
            Self::CCX | Self::CCZ => [R, R, R, B, A, B],
            Self::CX | Self::CZ | Self::Swap => [R, R, B, B, A, B],
            Self::X | Self::Z | Self::S | Self::Sdg => [R, B, B, B, A, B],
            // A reset frees a qubit; it must execute wherever it appears (no own condition).
            Self::R => [R, B, B, B, B, B],
            Self::Neg => [B, B, B, B, A, B],
            Self::Hmr => [R, B, B, R, A, B],
            Self::BitInvert | Self::BitStore0 | Self::BitStore1 => [B, B, B, R, A, B],
            Self::PushCondition => [B, B, B, B, R, B],
            Self::PopCondition => [B, B, B, B, B, B],
            Self::Givens => [R, R, B, B, A, R],
            // Nested DF (spec/DESIGN.md section 16.1): a register only; never conditioned.
            Self::Reflect => [B, B, B, B, B, R],
            // sos-sa (spec/SPEC-SA.md section 11): the control qubit only; may be conditioned.
            Self::SpinSwap | Self::SpinSwapDg => [R, B, B, B, A, B],
            Self::Segment => [B, B, B, B, B, R],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    Banned,
    Allowed,
    Required,
}

/// One op. Operand ids are `u32` in memory (`NONE` = absent); `ops.bin` stores them as `u64`
/// with `u64::MAX` for absent, and the reader rejects any other value that does not fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Op {
    pub kind: OperationType,
    pub q_control2: u32,
    pub q_control1: u32,
    pub q_target: u32,
    pub c_target: u32,
    pub c_condition: u32,
    pub r_target: u32,
}

impl Op {
    #[must_use]
    pub fn new(kind: OperationType) -> Self {
        Self {
            kind,
            q_control2: NONE,
            q_control1: NONE,
            q_target: NONE,
            c_target: NONE,
            c_condition: NONE,
            r_target: NONE,
        }
    }

    /// Qubit operands present on this op.
    pub fn qubits(&self) -> impl Iterator<Item = u32> {
        [self.q_target, self.q_control1, self.q_control2]
            .into_iter()
            .filter(|&q| q != NONE)
    }

    /// Checks operand shape and aliasing (ecdsa.fail's `validate`, returning an error instead of
    /// panicking). Proves: every operand the kind needs is present, no banned operand is, and no
    /// qubit appears twice in one op (aliasing would give free non-reversible resets).
    ///
    /// # Errors
    /// The first shape or aliasing violation.
    pub fn validate(&self) -> Result<(), String> {
        let k = self.kind.name();
        let (t, c1, c2) = (self.q_target, self.q_control1, self.q_control2);
        if (t != NONE && (t == c1 || t == c2)) || (c1 != NONE && c1 == c2) {
            return Err(format!("{k}: a qubit appears twice (q{t} q{c1} q{c2})"));
        }
        let names = [
            "q_target",
            "q_control1",
            "q_control2",
            "c_target",
            "c_condition",
            "r_target",
        ];
        let vals = [t, c1, c2, self.c_target, self.c_condition, self.r_target];
        for ((need, name), v) in self.kind.shape().into_iter().zip(names).zip(vals) {
            match need {
                Need::Required if v == NONE => return Err(format!("{k}: {name} is required")),
                Need::Banned if v != NONE => return Err(format!("{k}: {name} is not allowed")),
                _ => {}
            }
        }
        if self.kind == OperationType::AppendToRegister && (t == NONE) == (self.c_target == NONE) {
            return Err(format!("{k}: needs exactly one qubit or bit target"));
        }
        if self.kind == OperationType::Segment && self.r_target > SEG_INNER_END {
            return Err(format!("{k}: unknown segment code {}", self.r_target));
        }
        Ok(())
    }
}
