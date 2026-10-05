//! The structural check for the nested DF composition (spec/DESIGN.md section 16.3).
//!
//!
//! A circuit whose lane map is nested must have the shape `... 3 C 4 R 3 C' 4 ...` in `Segment`
//! hints, where `C` and `C'` are identical op for op (every field), `R` is exactly one `Reflect`
//! on exactly the inner uniform register and the only op between the copies, and nothing outside
//! the two copies names an inner uniform qubit. This is syntactic. The operator is certified by
//! the paired and diagonal lanes (`validate::run_nested`), not by this check. This check
//! certifies the architecture: one block encoding, a reflection, the same block encoding.
use super::compile::Layout;
use crate::circuit::{Op, OperationType as K, SEG_INNER_BEGIN, SEG_INNER_END};

/// The inner uniform register: uniform bits `lo .. lo + width`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inner {
    pub lo: u32,
    pub width: u32,
}

impl Inner {
    /// The inner register's qubit ids, in bit order.
    #[must_use]
    pub fn qubits(&self, l: &Layout) -> Vec<u32> {
        (0..self.width)
            .map(|j| 1 + l.system as u32 + self.lo + j)
            .collect()
    }
}

fn seg(op: &Op, code: u32) -> bool {
    op.kind == K::Segment && op.r_target == code
}

fn name(i: usize, op: &Op) -> String {
    format!("op {i} ({})", op.kind.name())
}

/// Checks the nested structure. `registers` are the compiled registers (final contents).
///
/// # Errors
/// The first rule the op stream breaks, named.
pub fn check_structure(
    ops: &[Op],
    l: &Layout,
    inner: Inner,
    registers: &[Vec<u32>],
) -> Result<(), String> {
    let at = |code: u32| -> Vec<usize> {
        ops.iter()
            .enumerate()
            .filter(|(_, op)| seg(op, code))
            .map(|(i, _)| i)
            .collect()
    };
    let (begins, ends) = (at(SEG_INNER_BEGIN), at(SEG_INNER_END));
    let reflects: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind == K::Reflect)
        .map(|(i, _)| i)
        .collect();
    if reflects.len() != 1 {
        return Err(format!(
            "nested: the circuit must contain exactly one Reflect, found {}",
            reflects.len()
        ));
    }
    let (&[b1, b2], &[e1, e2]) = (&begins[..], &ends[..]) else {
        return Err(format!(
            "nested: expected exactly two inner copies (Segment 3 ... Segment 4), found {} \
             Segment 3 and {} Segment 4",
            begins.len(),
            ends.len()
        ));
    };
    let r = reflects[0];
    if !(b1 < e1 && e1 < r && r < b2 && b2 < e2) {
        return Err("nested: the Reflect must lie between the two inner copies".into());
    }
    if r != e1 + 1 || b2 != r + 1 {
        return Err(format!(
            "nested: the Reflect (op {r}) must be the only op between the inner copies (ops {} \
             to {})",
            e1 + 1,
            b2 - 1
        ));
    }
    let (c1, c2) = (&ops[b1 + 1..e1], &ops[b2 + 1..e2]);
    if c1.len() != c2.len() {
        return Err(format!(
            "nested: the second inner copy has {} ops, the first {}; they must be identical",
            c2.len(),
            c1.len()
        ));
    }
    if let Some(d) = (0..c1.len()).find(|&d| c1[d] != c2[d]) {
        return Err(format!(
            "nested: the second inner copy differs from the first at its op {d}: {} vs {}",
            name(b2 + 1 + d, &c2[d]),
            name(b1 + 1 + d, &c1[d])
        ));
    }
    copy_rules(c1, b1 + 1)?;
    let qs = inner.qubits(l);
    // The Reflect: condition depth 0, on exactly the inner register.
    let depth = ops[..r]
        .iter()
        .map(|op| match op.kind {
            K::PushCondition => 1i64,
            K::PopCondition => -1,
            _ => 0,
        })
        .sum::<i64>();
    if depth != 0 {
        return Err(format!(
            "nested: the Reflect (op {r}) sits inside a condition block"
        ));
    }
    let reg = registers
        .get(ops[r].r_target as usize)
        .ok_or("nested: the Reflect's register is not declared")?;
    let mut got = reg.clone();
    got.sort_unstable();
    if got != qs {
        return Err(format!(
            "nested: the Reflect must act on exactly the inner uniform register (uniform bits \
             {}..{}, qubits {:?}), not on qubits {:?}",
            inner.lo,
            inner.lo + inner.width,
            qs,
            reg
        ));
    }
    // Nothing outside the copies acts on an inner qubit (declaring a register is not acting).
    for (i, op) in ops.iter().enumerate() {
        if (b1..=e1).contains(&i) || (b2..=e2).contains(&i) || i == r {
            continue;
        }
        let bookkeeping = matches!(op.kind, K::Register | K::AppendToRegister | K::DebugPrint);
        if !bookkeeping && op.qubits().any(|q| qs.contains(&q)) {
            return Err(format!(
                "nested: {} touches the inner uniform register outside the inner copies",
                name(i, op)
            ));
        }
        let reads_inner = op.kind == K::Givens
            && registers
                .get(op.r_target as usize)
                .is_some_and(|reg| reg.iter().any(|q| qs.contains(q)));
        if reads_inner {
            return Err(format!(
                "nested: {} reads the inner uniform register outside the inner copies",
                name(i, op)
            ));
        }
    }
    Ok(())
}

/// Inside a copy: no hints, register declarations or `Reflect`, and a balanced condition stack.
fn copy_rules(copy: &[Op], first: usize) -> Result<(), String> {
    let mut depth = 0i64;
    for (d, op) in copy.iter().enumerate() {
        match op.kind {
            K::Segment | K::Register | K::AppendToRegister | K::Reflect => {
                return Err(format!(
                    "nested: {} is not allowed inside an inner copy",
                    name(first + d, op)
                ))
            }
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Err(format!(
                "nested: {} pops a condition pushed outside the inner copy",
                name(first + d, op)
            ));
        }
    }
    if depth != 0 {
        return Err("nested: an inner copy leaves a condition block open".into());
    }
    Ok(())
}
