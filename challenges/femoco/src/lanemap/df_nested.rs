//! Lane-map family `df-nested-alias-v1`: the nested (Chebyshev) DF composition of von Burg et
//! al. (the nested DF form; no DF spec ships here).
//!
//! The uniform register is `u_o = k_o + mu_o` outer bits, then `w = k_i + mu_i` inner bits. The
//! outer bits pick an *item* by alias sampling (alias-v1 on the low `u_o` bits): a one-body term
//! (direct layout), the folded one-body item (folded layout) or a leaf `l`. The inner bits pick,
//! by that item's own alias table (all tables share `k_i` and `mu_i`), an inner index
//! `j = (k, s) = (j >> 1, j & 1)`: an eigenvector of the leaf, or a one-body eigenvector.
//!
//! A lane crosses the circuit's one `Reflect`: before it the inner register holds `a`, after it
//! `b` (spec/DESIGN.md section 16). The reference a control-1 lane must apply is
//! `M_{l,j(b)} M_{l,j(a)}` for a leaf, the one-body term itself (direct), or `M^T_{j(a)}`
//! (folded, first pass only). Counts are exact alias counts, and `rounding_error` is
//! `sum_t |c_t - c^_t|` over the flat spec's terms, in dyadic integers.
//!
//! Payload: `u32 layout` (0 direct, 1 folded), `u32 k_o, mu_o, k_i, mu_i`, `lambda_decl`
//! (`Exact::to_bytes`), `keep_o[2^k_o], alt_o[2^k_o]`, then per inner table `keep[2^k_i],
//! alt[2^k_i]`; there are `L` tables (direct) or `L + 1` (folded, table 0 is the one-body item).
use super::alias::{AliasMap, MAX_K, MAX_LAMBDA_EXPONENT};
use super::LaneMap;
use crate::spec::df::{DfForm, DfSpec};
use crate::spec::{EncodingSpec, Exact, Rotated, SystemOp};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

pub const FAMILY: &str = "df-nested-alias-v1";

/// Where the one-body terms live (the nested DF form; no DF spec ships here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneBody {
    /// Outer items `0..2N` are the one-body terms `(k, s)`; the inner copies do nothing there.
    Direct,
    /// Outer item 0 is the one-body operator, prepared on the inner register and applied in the
    /// first pass only (Lee et al. 2021 App. C).
    Folded,
}

/// What an outer lane names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OuterItem {
    OneBody { k: usize, spin: u8 },
    Folded,
    Leaf(usize),
}

/// One alias table: `k` index bits, `mu` keep bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub k: u32,
    pub mu: u32,
    pub keep: Vec<u32>,
    pub alt: Vec<u32>,
}

impl Table {
    /// An exact table realizing `counts` (summing to `2^(k + mu)`), by the alias-v1 builder.
    ///
    /// # Errors
    /// Counts that do not fit.
    pub fn from_counts(k: u32, mu: u32, counts: &[u64]) -> Result<Self, String> {
        let a = AliasMap::from_counts(k, mu, Exact::from_int(1), counts)?;
        Ok(Self {
            k,
            mu,
            keep: a.keep,
            alt: a.alt,
        })
    }

    /// The item lane value `v` (its low `k + mu` bits) names.
    #[must_use]
    pub fn item(&self, v: u64) -> usize {
        let i = (v & ((1u64 << self.k) - 1)) as usize;
        let r = (v >> self.k) & ((1u64 << self.mu) - 1);
        if r < u64::from(self.keep[i]) {
            i
        } else {
            self.alt[i] as usize
        }
    }

    /// Lanes per item, exactly.
    #[must_use]
    pub fn counts(&self, items: usize) -> Vec<u64> {
        let cap = 1u64 << self.mu;
        let mut n = vec![0u64; items.max(self.keep.len())];
        for (i, (&kp, &a)) in self.keep.iter().zip(&self.alt).enumerate() {
            n[i] += u64::from(kp);
            n[a as usize] += cap - u64::from(kp);
        }
        n.truncate(items);
        n
    }

    fn check(&self, items: usize, what: &str) -> Result<(), String> {
        let buckets = 1usize << self.k;
        if self.keep.len() != buckets || self.alt.len() != buckets {
            return Err(format!(
                "{FAMILY}: {what} table needs 2^k = {buckets} entries"
            ));
        }
        if let Some(i) = self.alt.iter().position(|&a| a as usize >= items) {
            return Err(format!(
                "{FAMILY}: {what} alt[{i}] = {} is not an item ({items} items)",
                self.alt[i]
            ));
        }
        if let Some(i) = (items..buckets).find(|&i| self.keep[i] != 0) {
            return Err(format!(
                "{FAMILY}: {what} bucket {i} is not an item and must have keep = 0"
            ));
        }
        if let Some(i) = self.keep.iter().position(|&v| u64::from(v) >> self.mu != 0) {
            return Err(format!("{FAMILY}: {what} keep[{i}] is not below 2^mu"));
        }
        Ok(())
    }
}

pub struct DfNestedMap {
    pub one_body: OneBody,
    pub lambda_decl: Exact,
    pub outer: Table,
    /// One table per leaf (direct), or the one-body item's table then one per leaf (folded).
    pub inner: Vec<Table>,
    /// Spatial orbitals and each leaf's eigenvector count, from the spec it was checked against.
    n: usize,
    xi: Vec<usize>,
}

fn df(spec: &dyn EncodingSpec) -> Result<&DfSpec, String> {
    let d = spec
        .as_any()
        .downcast_ref::<DfSpec>()
        .ok_or_else(|| format!("{FAMILY}: spec {} is not a df spec", spec.id()))?;
    if d.form != DfForm::Nested {
        return Err(format!(
            "{FAMILY}: spec {} is a flat-form df spec; use a *-df-nested-v1 spec",
            d.id
        ));
    }
    Ok(d)
}

impl DfNestedMap {
    /// Checks the tables against the spec (the nested DF form; no DF spec ships here, "Checks at parse").
    ///
    /// # Errors
    /// A flat-form spec, widths out of range, inner tables of different widths, a table of the
    /// wrong length, an alias target that is not an item, or a padding bucket that keeps lanes.
    pub fn new(
        one_body: OneBody,
        lambda_decl: Exact,
        outer: Table,
        inner: Vec<Table>,
        spec: &DfSpec,
    ) -> Result<Self, String> {
        if spec.form != DfForm::Nested {
            return Err(format!(
                "{FAMILY}: spec {} is a flat-form df spec; use a *-df-nested-v1 spec",
                spec.id
            ));
        }
        let (n, xi) = (
            spec.n,
            spec.leaves.iter().map(|l| l.e.len()).collect::<Vec<_>>(),
        );
        let first = inner.first().ok_or(format!("{FAMILY}: no inner tables"))?;
        let (k_i, mu_i) = (first.k, first.mu);
        let bits = [outer.k, outer.mu, k_i, mu_i];
        if outer.k > MAX_K || k_i > MAX_K || outer.mu > 31 || mu_i > 31 {
            return Err(format!("{FAMILY}: widths {bits:?} out of range"));
        }
        if outer.k + outer.mu + k_i + mu_i > 63 {
            return Err(format!(
                "{FAMILY}: u = k_o + mu_o + k_i + mu_i = {} above 63",
                bits.iter().sum::<u32>()
            ));
        }
        if let Some(t) = inner.iter().position(|t| (t.k, t.mu) != (k_i, mu_i)) {
            return Err(format!(
                "{FAMILY}: inner table {t} has (k, mu) = ({}, {}) but table 0 has ({k_i}, {mu_i}); \
                 every inner table must share one width",
                inner[t].k, inner[t].mu
            ));
        }
        let (outer_items, tables) = match one_body {
            OneBody::Direct => (2 * n + xi.len(), xi.len()),
            OneBody::Folded => (1 + xi.len(), 1 + xi.len()),
        };
        if inner.len() != tables {
            return Err(format!(
                "{FAMILY}: {} inner tables, the spec needs {tables}",
                inner.len()
            ));
        }
        outer.check(outer_items, "outer")?;
        for (t, table) in inner.iter().enumerate() {
            let items = match (one_body, t) {
                (OneBody::Folded, 0) => 2 * n,
                (OneBody::Folded, t) => 2 * xi[t - 1],
                (OneBody::Direct, t) => 2 * xi[t],
            };
            table.check(items, &format!("inner table {t}"))?;
        }
        if lambda_decl.dyadic_exponent().is_none() || lambda_decl <= Exact::zero() {
            return Err(format!(
                "{FAMILY}: lambda_decl {lambda_decl} must be positive and dyadic"
            ));
        }
        // As for alias-v1: the rounding error aligns every flat term (2.2M for
        // reiher) to lambda_decl's exponent, so an unbounded one is a denial of service.
        if lambda_decl
            .dyadic_exponent()
            .is_some_and(|e| e > MAX_LAMBDA_EXPONENT)
        {
            return Err(format!(
                "{FAMILY}: lambda_decl exponent above {MAX_LAMBDA_EXPONENT}"
            ));
        }
        Ok(Self {
            one_body,
            lambda_decl,
            outer,
            inner,
            n,
            xi,
        })
    }

    /// Outer bits `u_o = k_o + mu_o`: the inner register starts here.
    #[must_use]
    pub fn outer_bits(&self) -> u32 {
        self.outer.k + self.outer.mu
    }

    /// Inner register width `w = k_i + mu_i`.
    #[must_use]
    pub fn inner_width(&self) -> u32 {
        self.inner[0].k + self.inner[0].mu
    }

    /// The outer item lane `s` names.
    #[must_use]
    pub fn decode_outer(&self, s: u64) -> OuterItem {
        let o = self.outer.item(s);
        match self.one_body {
            OneBody::Direct if o < 2 * self.n => OuterItem::OneBody {
                k: o >> 1,
                spin: (o & 1) as u8,
            },
            OneBody::Direct => OuterItem::Leaf(o - 2 * self.n),
            OneBody::Folded if o == 0 => OuterItem::Folded,
            OneBody::Folded => OuterItem::Leaf(o - 1),
        }
    }

    /// The inner table an outer item uses, if any (direct one-body terms use none).
    #[must_use]
    pub fn table_of(&self, item: OuterItem) -> Option<usize> {
        match (self.one_body, item) {
            (_, OuterItem::OneBody { .. }) => None,
            (_, OuterItem::Folded) => Some(0),
            (OneBody::Direct, OuterItem::Leaf(l)) => Some(l),
            (OneBody::Folded, OuterItem::Leaf(l)) => Some(l + 1),
        }
    }

    /// Inner index `j` (so `(k, s) = (j >> 1, j & 1)`) that inner value `a` names in `table`.
    #[must_use]
    pub fn inner_item(&self, table: usize, a: u64) -> usize {
        self.inner[table].item(a)
    }

    fn operator(&self, d: &DfSpec, s: u64, after: u64) -> SystemOp {
        let u_o = self.outer_bits();
        let (a, b) = (s >> u_o, after >> u_o);
        let item = self.decode_outer(s);
        match (item, self.table_of(item)) {
            (OuterItem::OneBody { k, spin }, _) => d.one_body_op(k, spin),
            (OuterItem::Folded, Some(t)) => {
                let j = self.inner_item(t, a);
                d.one_body_op(j >> 1, (j & 1) as u8)
            }
            (OuterItem::Leaf(l), Some(t)) => {
                let (ja, jb) = (self.inner_item(t, a), self.inner_item(t, b));
                product(
                    d.inner_op(l, jb >> 1, (jb & 1) as u8),
                    d.inner_op(l, ja >> 1, (ja & 1) as u8),
                )
            }
            _ => unreachable!("every folded or leaf item has a table"),
        }
    }
}

/// `x y` for two rotated operators (`y` acts first).
fn product(x: SystemOp, y: SystemOp) -> SystemOp {
    match (x, y) {
        (SystemOp::Rotated(x), SystemOp::Rotated(y)) => SystemOp::Rotated(Rotated {
            phase: (x.phase + y.phase) % 4,
            parts: x.parts.into_iter().chain(y.parts).collect(),
        }),
        _ => unreachable!("df inner operators are rotated"),
    }
}

/// `m / 2^e` without gcd reduction, for fast exact sums of many dyadics.
#[derive(Clone)]
struct Dy {
    m: BigInt,
    e: u32,
}

impl Dy {
    fn of(x: &Exact) -> Result<Self, String> {
        let e = x
            .dyadic_exponent()
            .ok_or(format!("{FAMILY}: value is not dyadic"))?;
        Ok(Self {
            m: x.num.clone(),
            e,
        })
    }
    fn at(&self, e: u32) -> BigInt {
        &self.m << (e - self.e)
    }
}

/// Accumulates `sum |c - c^|` at a fixed exponent.
struct ErrSum {
    e: u32,
    total: BigInt,
}

impl ErrSum {
    fn add(&mut self, c: &BigInt, got: &BigInt) {
        self.total += (c - got).abs();
    }
}

fn exact(x: f64) -> Exact {
    Exact::from_f64(x).unwrap_or_else(|_| Exact::zero())
}

impl LaneMap for DfNestedMap {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.outer_bits() + self.inner_width()
    }
    fn lambda_decl(&self) -> Exact {
        self.lambda_decl.clone()
    }
    /// The diagonal reference (second-pass value equal to the first).
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        self.reference_nested(spec, s, s)
    }
    fn reference_nested(&self, spec: &dyn EncodingSpec, s: u64, after: u64) -> SystemOp {
        match df(spec) {
            Ok(d) => self.operator(d, s, after),
            Err(_) => SystemOp::Monomial(crate::spec::Monomial {
                phase: 0,
                majoranas: Vec::new(),
            }),
        }
    }
    fn inner_bits(&self) -> Option<(u32, u32)> {
        Some((self.outer_bits(), self.inner_width()))
    }
    /// `sum_t |c_t - c^_t|` over the flat spec's terms (spec/DESIGN.md section 16.5):
    /// one-body `c = |t_k|/2`, two-body `c = |e_k1 e_k2|/8` for `(k1,s1) != (k2,s2)`, against
    /// `c^ = lambda n_o / 2^u_o` (direct one-body), `lambda n_T m_j / 2^(u_o + w)` (folded) and
    /// `2 lambda n_l m_j1 m_j2 / 2^(u_o + 2w)` (leaf). The `j1 = j2` products and each leaf's
    /// `-1` are multiples of the identity: an energy shift, not an error.
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let d = df(spec)?;
        if d.n != self.n
            || d.leaves
                .iter()
                .map(|l| l.e.len())
                .ne(self.xi.iter().copied())
        {
            return Err(format!("{FAMILY}: lane map was built for a different spec"));
        }
        let (u_o, w) = (self.outer_bits(), self.inner_width());
        let lam = Dy::of(&self.lambda_decl)?;
        let t: Vec<Dy> =
            d.t.iter()
                .map(|&x| Dy::of(&exact(x).abs()))
                .collect::<Result<_, _>>()?;
        let e: Vec<Vec<Dy>> = d
            .leaves
            .iter()
            .map(|l| l.e.iter().map(|&x| Dy::of(&exact(x).abs())).collect())
            .collect::<Result<_, _>>()?;
        // Common exponent: c terms are t/2 (e_t + 1) and e1 e2 / 8 (e_1 + e_2 + 3); encoded
        // terms are lambda * integer / 2^(u_o + 2w + 1) at most.
        let et = t.iter().map(|x| x.e + 1).max().unwrap_or(0);
        let ee = e.iter().flatten().map(|x| x.e).max().unwrap_or(0);
        let ex = et.max(2 * ee + 3).max(lam.e + u_o + 2 * w);
        let mut err = ErrSum {
            e: ex,
            total: BigInt::zero(),
        };
        let outer = self.outer.counts(match self.one_body {
            OneBody::Direct => 2 * self.n + self.xi.len(),
            OneBody::Folded => 1 + self.xi.len(),
        });
        // lambda * m / 2^shift at exponent ex (exact: ex >= lam.e + u_o + 2w >= lam.e + shift).
        let enc = |m: &BigInt, shift: u32| (&lam.m * m) << (ex - lam.e - shift);
        let half_t: Vec<BigInt> = t.iter().map(|x| x.at(ex - 1)).collect();
        match self.one_body {
            OneBody::Direct => {
                for (o, &n_o) in outer.iter().enumerate().take(2 * self.n) {
                    err.add(&half_t[o >> 1], &enc(&BigInt::from(n_o), u_o));
                }
            }
            OneBody::Folded => {
                let m = self.inner[0].counts(2 * self.n);
                for (j, &m_j) in m.iter().enumerate() {
                    let nm = BigInt::from(outer[0]) * m_j;
                    err.add(&half_t[j >> 1], &enc(&nm, u_o + w));
                }
            }
        }
        let first_leaf = usize::from(self.one_body == OneBody::Folded);
        let leaf_base = match self.one_body {
            OneBody::Direct => 2 * self.n,
            OneBody::Folded => 1,
        };
        for (l, el) in e.iter().enumerate() {
            let n_l = outer[leaf_base + l];
            let m = self.inner[first_leaf + l].counts(2 * el.len());
            // c = |e_k1 e_k2| / 8 at exponent ex: a_k1 a_k2 with a_k = |e_k| at (ex - 3 + e_k).
            let a: Vec<&BigInt> = el.iter().map(|x| &x.m).collect();
            let two_n = BigInt::from(2 * n_l);
            for j1 in 0..m.len() {
                let kn = &two_n * m[j1];
                for j2 in 0..m.len() {
                    if j1 == j2 {
                        continue;
                    }
                    let (x, y) = (&el[j1 >> 1], &el[j2 >> 1]);
                    let c = (a[j1 >> 1] * a[j2 >> 1]) << (ex - 3 - x.e - y.e);
                    err.add(&c, &enc(&(&kn * m[j2]), u_o + 2 * w));
                }
            }
        }
        Ok(Exact::dyadic(err.total, err.e))
    }
    fn alias_bits(&self) -> Option<(u32, u32)> {
        Some((self.outer.k, self.outer.mu))
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = super::MAGIC.to_vec();
        out.extend_from_slice(&u16::try_from(FAMILY.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(FAMILY.as_bytes());
        let layout: u32 = match self.one_body {
            OneBody::Direct => 0,
            OneBody::Folded => 1,
        };
        let (k_i, mu_i) = (self.inner[0].k, self.inner[0].mu);
        for v in [layout, self.outer.k, self.outer.mu, k_i, mu_i] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.lambda_decl.to_bytes());
        for t in std::iter::once(&self.outer).chain(&self.inner) {
            for v in t.keep.iter().chain(&t.alt) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// Parses a `df-nested-alias-v1` payload and checks it against the nested df spec.
///
/// # Errors
/// A malformed payload, a spec that is not a nested-form df spec, or an inconsistent table.
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let d = df(spec)?;
    let u32_at = |at: usize| -> Result<u32, String> {
        let s = payload
            .get(at..at + 4)
            .ok_or(format!("{FAMILY}: truncated"))?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let one_body = match u32_at(0)? {
        0 => OneBody::Direct,
        1 => OneBody::Folded,
        v => return Err(format!("{FAMILY}: unknown one-body layout {v}")),
    };
    let (k_o, mu_o, k_i, mu_i) = (u32_at(4)?, u32_at(8)?, u32_at(12)?, u32_at(16)?);
    if k_o > MAX_K || k_i > MAX_K {
        return Err(format!("{FAMILY}: k_o = {k_o} or k_i = {k_i} out of range"));
    }
    let (lambda_decl, used) =
        Exact::from_bytes(payload.get(20..).ok_or(format!("{FAMILY}: truncated"))?)?;
    let mut at = 20 + used;
    let tables = match one_body {
        OneBody::Direct => d.leaves.len(),
        OneBody::Folded => 1 + d.leaves.len(),
    };
    let want = at + 8 * (1usize << k_o) + tables * 8 * (1usize << k_i);
    if payload.len() != want {
        return Err(format!(
            "{FAMILY}: payload length {} does not match the widths and the spec ({want})",
            payload.len()
        ));
    }
    let mut table = |k: u32, mu: u32| -> Result<Table, String> {
        let b = 1usize << k;
        let read = |off: usize| -> Result<Vec<u32>, String> {
            (0..b).map(|i| u32_at(off + 4 * i)).collect()
        };
        let t = Table {
            k,
            mu,
            keep: read(at)?,
            alt: read(at + 4 * b)?,
        };
        at += 8 * b;
        Ok(t)
    };
    let outer = table(k_o, mu_o)?;
    let inner = (0..tables)
        .map(|_| table(k_i, mu_i))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Box::new(DfNestedMap::new(
        one_body,
        lambda_decl,
        outer,
        inner,
        d,
    )?))
}

/// Float64 largest-remainder counts toward `weights`, summing to `2^bits` (a helper for
/// builders; the harness trusts only the exact rounding error).
fn lr_counts(weights: &[f64], bits: u32) -> Vec<u64> {
    let lanes = 1u64 << bits;
    let total: f64 = weights.iter().sum();
    let ideal: Vec<f64> = weights.iter().map(|w| w / total * lanes as f64).collect();
    let mut counts: Vec<u64> = ideal.iter().map(|x| x.floor() as u64).collect();
    let short = lanes.saturating_sub(counts.iter().sum());
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|&a, &b| (ideal[b] - ideal[b].floor()).total_cmp(&(ideal[a] - ideal[a].floor())));
    for &i in order
        .iter()
        .cycle()
        .take(usize::try_from(short).unwrap_or(0))
    {
        counts[i] += 1;
    }
    counts
}

/// Builds a nested lane map for `spec` (the nested DF form; no DF spec ships here, "Constructor"): outer
/// weights `|t_k|/2` per one-body term (direct) or `lambda_T` (folded), and `S_l^2/4` per leaf;
/// inner weights `|e_lk|` (or `|t_k|`) per `(k, s)`; float64 largest-remainder counts, Walker
/// tables, and `lambda_decl` = the spec's nested lambda, exactly.
///
/// # Errors
/// More items than buckets, `u > 63`, or a flat-form spec.
pub fn build(
    spec: &DfSpec,
    one_body: OneBody,
    (k_o, mu_o): (u32, u32),
    (k_i, mu_i): (u32, u32),
) -> Result<DfNestedMap, String> {
    let s_l = |e: &[f64]| e.iter().map(|x| x.abs()).sum::<f64>();
    let leaf_w = spec.leaves.iter().map(|l| s_l(&l.e).powi(2) / 4.0);
    let outer_w: Vec<f64> = match one_body {
        OneBody::Direct => spec
            .t
            .iter()
            .flat_map(|t| [t.abs() / 2.0; 2])
            .chain(leaf_w)
            .collect(),
        OneBody::Folded => std::iter::once(s_l(&spec.t)).chain(leaf_w).collect(),
    };
    if outer_w.len() > 1 << k_o {
        return Err(format!(
            "{FAMILY}: {} outer items do not fit 2^{k_o}",
            outer_w.len()
        ));
    }
    let outer = Table::from_counts(k_o, mu_o, &lr_counts(&outer_w, k_o + mu_o))?;
    let per_spin = |v: &[f64]| -> Vec<f64> { v.iter().flat_map(|x| [x.abs(); 2]).collect() };
    let mut inner_w: Vec<Vec<f64>> = Vec::new();
    if one_body == OneBody::Folded {
        inner_w.push(per_spin(&spec.t));
    }
    inner_w.extend(spec.leaves.iter().map(|l| per_spin(&l.e)));
    let inner = inner_w
        .iter()
        .map(|w| {
            if w.len() > 1 << k_i {
                return Err(format!(
                    "{FAMILY}: {} inner items do not fit 2^{k_i}",
                    w.len()
                ));
            }
            Table::from_counts(k_i, mu_i, &lr_counts(w, k_i + mu_i))
        })
        .collect::<Result<Vec<_>, _>>()?;
    DfNestedMap::new(one_body, spec.lambda.clone(), outer, inner, spec)
}
