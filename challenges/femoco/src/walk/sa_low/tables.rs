//! The classical data the Low et al. walk reads: the outer alias buckets (outer PREPARE), the
//! inner alias buckets of every square (inner PREPARE, inside each copy) and the Givens angles
//! of every network (RPREP, inside each copy).
//!
//! **Network index.** One register `idx = lo | hi << k_i` names the network a lane rotates by:
//! `lo` holds `k_i` bits and `hi` the rest.
//! - square `(r, c)`, inner item `b < B`: `lo = b`, `hi = r`, network `nets[r B + b]`;
//!   the identity item `b = B` (and the padding `B < lo < 2^k_i`) reads all-zero angles;
//! - one-body eigenvector `r`: `idx = R 2^k_i + r`, network `e_nets[r]`.
//!
//! So a square's `hi` is its `r` from the outer data, and its `lo` is the inner alias output;
//! for a one-body lane both halves come from the outer data (the inner lookup reads 0 there).
//! This is Low et al.'s RPREP index `(r, b)` (their `N + R B` entries), padded to `2^k_i` per
//! `r`.
//!
//! **Outer item data** (`D = k_i + h + q_b + 2` bits, low first): `lo | hi | q | is_ob | pos_e`.
//! One-body `r`: `lo = r mod 2^k_i`, `hi = R + r div 2^k_i`, `q` all ones (past every square, so
//! the inner lookup reads nothing), `is_ob = 1`, `pos_e = [e_r >= 0]`. Square `(r, c)`: `lo = 0`,
//! `hi = r`, `q = r C + c`, `is_ob = pos_e = 0`.
//!
//! **Outer bucket word** (`mu_o + 2 D` bits): `keep_o[i] | data(i) | data(alt_o[i])`, own data
//! zero for padding buckets (their keep is 0, so the alt is always chosen).
//!
//! **Inner bucket word** (`mu_i + 2 + k_i + 2` bits), at index `x = q 2^k_i + i` for square `q`:
//! `keep | own_neg | own_id | alt_b | alt_neg | alt_id`. The own item is the bucket `i` itself,
//! whose `b` is the lane's inner index bits (copied, not looked up). `neg` is the sign bit of the
//! term: `M_(b, s) = -sign(w_b) Z(u_b, s)` gives `neg = [w_b > 0]`, `M_B = sign(wB) I` gives
//! `neg = [wB < 0]`. `id = [b = B]`.
//!
//! **Angle word** (the sum of the rotation widths): rotation `j`'s angle at its slot. Rotations
//! whose angles all lie below `2^(beta - 1)` get `beta - 1`-qubit registers (measured from the
//! data, as `shared::angles` does). On a tapered spec (spec/SPEC-SA.md section 13) rotation `j`
//! holds only the top `widths[j]` bits of its angle ([`stored_angle`]), so its register is
//! measured from those.
use crate::lanemap::sa_nested::SaNestedMap;
use crate::spec::sa::SaSpec;
use crate::spec::Network;
use crate::walk::shared::arith::bits_for_value;
use crate::walk::shared::lookup::{put, word, Word};

/// The value rotation `j`'s angle register holds for the spec's `beta`-bit angle `a`: `a` itself,
/// or on a tapered spec (spec/SPEC-SA.md section 13) its top `widths[j]` bits, which is how the
/// harness reads a `Givens` at chain position `j` there (and charges it `2 (widths[j] - 2)`).
#[must_use]
pub fn stored_angle(spec: &SaSpec, j: usize, a: u32) -> u64 {
    let unit = 1u32 << spec.beta;
    let shift = spec
        .widths
        .as_ref()
        .map_or(0, |w| u32::from(spec.beta - w[j]));
    u64::from((a % unit) >> shift)
}

/// Bits needed to hold every value in `0..n`.
#[must_use]
pub fn bits_for(n: u64) -> usize {
    (u64::BITS - n.saturating_sub(1).leading_zeros()).max(1) as usize
}

/// The angle slots of a streamed load: `g` rotations are held at once, rotation `j` in slot
/// `j mod g` while chunk `j div g` is loaded. `g = N - 1` (one chunk) is the published layout.
#[derive(Clone, Debug)]
pub struct Slots {
    pub g: usize,
    pub chunks: usize,
    /// Width of slot `s`: the widest rotation that uses it.
    pub width: Vec<usize>,
    pub at: Vec<usize>,
}

impl Slots {
    /// Total slot qubits.
    #[must_use]
    pub fn bits(&self) -> usize {
        self.width.iter().sum()
    }

    /// The chunk order of one conjugation: `C-1, ..., 0` for `V^dagger`, then `1, ..., C-1`
    /// for `V` (chunk 0 serves both halves around the Majorana).
    #[must_use]
    pub fn states(&self) -> Vec<usize> {
        (0..self.chunks).rev().chain(1..self.chunks).collect()
    }
}

/// Widths and data of the walk's tables. One struct serves every variant; each constructor
/// sets the fields of its own variant and leaves the others at the published layout:
/// [`SaTables::with_index`] (`sa-lowq`'s dense index), [`SaTables::with`] and
/// [`SaTables::with_items`] (`sa-toff`'s levers), [`SaTables::with_layout`] (`sa-pareto`'s lean
/// layout and chunks).
pub struct SaTables<'a> {
    pub spec: &'a SaSpec,
    pub map: &'a SaNestedMap,
    /// Inner index bits `k_i` (= the `lo` field).
    pub k_i: usize,
    /// Bits of `hi`.
    pub h: usize,
    /// Bits of the square number `q` (all ones marks a one-body lane).
    pub q_b: usize,
    /// Rotations per network (`N - 1`) and their mode pairs `(p, q)` (spatial orbitals).
    pub modes: Vec<(usize, usize)>,
    /// Register width rotation `j` needs (the bits of its largest angle over all networks).
    pub widths: Vec<usize>,
    /// Start of rotation `j` in the angle word.
    pub at: Vec<usize>,
    /// Dense network index (`sa-lowq`): `v = r B + b` for square networks and `R B + r` for
    /// one-body ones, `E = R B + N` values, instead of `lo | hi << k_i` (module docs).
    pub dense: bool,
    /// The inner word stores only the sign of each item; `id = [b = B]` is computed from `b`
    /// (`Tweaks::derive_id`).
    pub derive_id: bool,
    /// The outer data drops `lo`: a one-body item's `q` is `ones | r`, whose low `k_i` bits are
    /// `r mod 2^k_i` (`Tweaks::compact_outer`).
    pub compact: bool,
    /// Low bits of a one-body `q` that hold `r` (compact layout).
    pub kb: usize,
    /// Item layout (`Tweaks::item_outer`): the outer word stores `hi | is_ob | pos_e` for the
    /// own item (whose index is the bucket's, copied from the lookup index) and `item | hi |
    /// is_ob | pos_e` for the alt; the inner lookup is indexed by `item 2^k_i + i`.
    pub item_layout: bool,
    /// Bits of an outer item index (`N + R C` items).
    pub kx: usize,
    /// Lean layout (`sa-pareto`): the outer item carries no `is_ob | pos_e` (they are flag bits
    /// of the angle word instead), the inner word no `id` flags (computed from `b`), and the
    /// RPREP iteration skips its left-only ANDs ([`SaTables::leaf`]).
    pub lean: bool,
    /// `sa-pareto`'s angle chunks `C` and rotations per chunk `g = ceil((N - 1) / C)`: slot `s`
    /// holds rotation `c g + s` while chunk `c` is loaded (1 and `N - 1` for every other
    /// variant; `sa-lowq` streams through [`SaTables::slots`] instead).
    pub chunks: usize,
    pub g: usize,
    /// Width and start of each slot in the angle register.
    pub slot_w: Vec<usize>,
    pub slot_at: Vec<usize>,
    /// Flag bits after the slots: `is_ob | pos_e` when lean, none otherwise.
    pub angle_flags: usize,
}

/// Outer data field offsets (`lo` is absent in the compact layout and with the dense index).
#[derive(Clone, Copy, Debug)]
pub struct Fields {
    pub lo: Option<usize>,
    pub hi: usize,
    pub q: usize,
    pub is_ob: usize,
    pub pos_e: usize,
    pub end: usize,
}

/// Which layout a [`SaTables`] is built with (all off: the published one).
#[derive(Clone, Copy, Default)]
struct Layout {
    dense: bool,
    derive_id: bool,
    compact: bool,
    lean: bool,
    chunks: usize,
}

fn starts(widths: &[usize]) -> Vec<usize> {
    widths
        .iter()
        .scan(0, |acc, &w| {
            let a = *acc;
            *acc += w;
            Some(a)
        })
        .collect()
}

impl<'a> SaTables<'a> {
    /// # Panics
    /// If the networks are not one shared chain, or the inner width is too small.
    #[must_use]
    pub fn new(spec: &'a SaSpec, map: &'a SaNestedMap) -> Self {
        Self::build(spec, map, Layout::default())
    }

    /// [`SaTables::new`] with the dense network index when `dense` (`sa-lowq`).
    ///
    /// # Panics
    /// As [`SaTables::new`].
    #[must_use]
    pub fn with_index(spec: &'a SaSpec, map: &'a SaNestedMap, dense: bool) -> Self {
        Self::build(
            spec,
            map,
            Layout {
                dense,
                ..Layout::default()
            },
        )
    }

    /// [`SaTables::new`] with `sa-toff`'s derived `id` and compact outer data.
    ///
    /// # Panics
    /// As [`SaTables::new`].
    #[must_use]
    pub fn with(spec: &'a SaSpec, map: &'a SaNestedMap, derive_id: bool, compact: bool) -> Self {
        Self::build(
            spec,
            map,
            Layout {
                derive_id,
                compact,
                ..Layout::default()
            },
        )
    }

    /// Tables for `sa-pareto`'s lean layout and/or `chunks` angle chunks.
    ///
    /// # Panics
    /// As [`SaTables::new`].
    #[must_use]
    pub fn with_layout(spec: &'a SaSpec, map: &'a SaNestedMap, lean: bool, chunks: usize) -> Self {
        Self::build(
            spec,
            map,
            Layout {
                lean,
                chunks,
                ..Layout::default()
            },
        )
    }

    fn build(spec: &'a SaSpec, map: &'a SaNestedMap, l: Layout) -> Self {
        assert!(
            !(l.lean && l.dense),
            "the lean layout is not combined with the dense index"
        );
        let k_i = map.inner[0].k as usize;
        assert!(spec.b < 1 << k_i, "the identity item needs b = B < 2^k_i");
        let hi_max = spec.r as u64 + ((spec.n as u64 - 1) >> k_i);
        let h = if l.dense {
            bits_for((spec.r * spec.b + spec.n) as u64)
        } else {
            bits_for(hi_max + 1)
        };
        let rc = (spec.r * spec.c) as u64;
        let kb = bits_for(spec.n as u64).max(k_i);
        let mut q_b = bits_for(rc + 1);
        if l.compact {
            // One-body codes `ones | r` must lie past every square.
            while q_b <= kb || (1u64 << q_b) - (1u64 << kb) < rc {
                q_b += 1;
            }
        }
        let chain = |n: &Network| -> Vec<(usize, usize)> {
            n.rotations
                .iter()
                .map(|&(p, q, _)| (usize::from(p), usize::from(q)))
                .collect()
        };
        let modes = chain(&spec.e_nets[0]);
        let all: Vec<&Network> = spec
            .e_nets
            .iter()
            .chain(&spec.nets)
            .map(AsRef::as_ref)
            .collect();
        assert!(
            all.iter().all(|n| chain(n) == modes),
            "every network must be the same chain of rotations"
        );
        let widths: Vec<usize> = (0..modes.len())
            .map(|j| {
                let top = all
                    .iter()
                    .map(|n| stored_angle(spec, j, n.rotations[j].2))
                    .max()
                    .unwrap_or(0);
                bits_for_value(top)
            })
            .collect();
        let at = starts(&widths);
        let r = modes.len();
        let chunks = l.chunks.clamp(1, r.max(1));
        let g = r.div_ceil(chunks);
        let chunks = r.div_ceil(g);
        let slot_w: Vec<usize> = (0..g)
            .map(|s| (s..r).step_by(g).map(|j| widths[j]).max().unwrap_or(1))
            .collect();
        let slot_at = starts(&slot_w);
        Self {
            spec,
            map,
            k_i,
            h,
            q_b,
            modes,
            widths,
            at,
            dense: l.dense,
            derive_id: l.derive_id,
            compact: l.compact,
            kb,
            item_layout: false,
            kx: bits_for(spec.outer_items() as u64),
            lean: l.lean,
            chunks,
            g,
            slot_w,
            slot_at,
            angle_flags: if l.lean { 2 } else { 0 },
        }
    }

    /// `sa-pareto`'s tables ([`SaTables::with_layout`]), with the compact outer data when
    /// `compact` (`pareto::Narrow`'s `c`: no `lo` field; a one-body item's `q` is `ones | r`).
    /// With `compact` off this is [`SaTables::with_layout`].
    ///
    /// # Panics
    /// As [`SaTables::new`].
    #[must_use]
    pub fn with_pareto(
        spec: &'a SaSpec,
        map: &'a SaNestedMap,
        lean: bool,
        chunks: usize,
        compact: bool,
    ) -> Self {
        Self::build(
            spec,
            map,
            Layout {
                lean,
                chunks,
                compact,
                ..Layout::default()
            },
        )
    }

    /// [`SaTables::with`] in the item layout (implies the compact one-body encoding of `b`).
    #[must_use]
    pub fn with_items(spec: &'a SaSpec, map: &'a SaNestedMap, derive_id: bool) -> Self {
        let mut t = Self::with(spec, map, derive_id, true);
        t.item_layout = true;
        assert!(t.kx >= t.k_i, "one-body items need k_i low bits");
        t
    }

    /// Bits of the outer data's `lo` field: `k_i`, or none with the dense index (the whole
    /// network base is then the `hi` field).
    #[must_use]
    pub fn lo_bits(&self) -> usize {
        if self.dense {
            0
        } else {
            self.k_i
        }
    }

    /// Item layout: the selected-item register `item | hi | is_ob | pos_e` (low first); the
    /// own part of the word is its last `h + 2` bits.
    #[must_use]
    pub fn item_fields(&self) -> Fields {
        let hi = self.kx;
        let is_ob = hi + self.h;
        Fields {
            lo: None,
            hi,
            q: 0,
            is_ob,
            pos_e: is_ob + 1,
            end: is_ob + 2,
        }
    }

    /// Item layout: `hi | is_ob | pos_e` of `item`.
    #[must_use]
    pub fn item_flags(&self, item: usize) -> u64 {
        let s = self.spec;
        let h = self.h;
        if item < s.n {
            let r = item as u64;
            (s.r as u64 + (r >> self.k_i)) | 1 << h | u64::from(s.e[item] >= 0.0) << (h + 1)
        } else {
            ((item - s.n) / s.c) as u64
        }
    }

    /// Item layout outer word of bucket `i`: `keep | own flags | alt item | alt flags`.
    #[must_use]
    pub fn item_word(&self, i: u64) -> Word {
        let o = &self.map.outer;
        let (mu, f) = (o.mu as usize, self.h + 2);
        let mut w = word(mu + 2 * f + self.kx);
        let Ok(idx) = usize::try_from(i) else {
            return w;
        };
        if idx >= o.keep.len() {
            return w;
        }
        put(&mut w, 0, mu, u64::from(o.keep[idx]));
        if idx < self.spec.outer_items() {
            put(&mut w, mu, f, self.item_flags(idx));
        }
        let alt = o.alt[idx] as usize;
        put(&mut w, mu + f, self.kx, alt as u64);
        put(&mut w, mu + f + self.kx, f, self.item_flags(alt));
        w
    }

    /// Item layout inner lookup range `N 2^k_i .. (N + R C) 2^k_i`.
    #[must_use]
    pub fn item_range(&self) -> (u64, u64) {
        let s = self.spec;
        (
            (s.n as u64) << self.k_i,
            (s.outer_items() as u64) << self.k_i,
        )
    }

    /// Item layout inner word at `x = item 2^k_i + i`.
    #[must_use]
    pub fn item_inner_word(&self, x: u64) -> Word {
        let (start, _) = self.item_range();
        if x < start {
            return word(self.inner_word_bits());
        }
        self.inner_word(x - start)
    }

    /// Bits of one outer item's data.
    #[must_use]
    pub fn outer_data_bits(&self) -> usize {
        self.fields().end
    }

    /// Field offsets in the outer data: `lo | hi | q | is_ob | pos_e` (no `lo` with the dense
    /// index, no `is_ob | pos_e` in the lean layout), or `q | hi | is_ob | pos_e` (compact).
    #[must_use]
    pub fn fields(&self) -> Fields {
        if self.compact {
            let hi = self.q_b;
            let is_ob = hi + self.h;
            return Fields {
                lo: None,
                hi,
                q: 0,
                is_ob,
                pos_e: is_ob + 1,
                end: if self.lean { is_ob } else { is_ob + 2 },
            };
        }
        let hi = self.lo_bits();
        let q = hi + self.h;
        let is_ob = q + self.q_b;
        Fields {
            lo: (!self.dense).then_some(0),
            hi,
            q,
            is_ob,
            pos_e: is_ob + 1,
            end: if self.lean { is_ob } else { is_ob + 2 },
        }
    }

    /// Field offsets in the outer data: `(lo, hi, q, is_ob, pos_e)` (non-compact layout).
    #[must_use]
    pub fn outer_fields(&self) -> (usize, usize, usize, usize, usize) {
        let f = self.fields();
        (f.lo.unwrap_or(0), f.hi, f.q, f.is_ob, f.pos_e)
    }

    /// The outer data of `item`.
    #[must_use]
    pub fn outer_data(&self, item: usize) -> u64 {
        let s = self.spec;
        let f = self.fields();
        let (hi_at, q_at, ob_at, pos_at) = (f.hi, f.q, f.is_ob, f.pos_e);
        let mask = (1u64 << self.k_i) - 1;
        if self.dense {
            let base = if item < s.n {
                (s.r * s.b + item) as u64
            } else {
                ((item - s.n) / s.c * s.b) as u64
            };
            let rest = if item < s.n {
                ((1u64 << self.q_b) - 1) << q_at
                    | 1 << ob_at
                    | u64::from(s.e[item] >= 0.0) << pos_at
            } else {
                ((item - s.n) as u64) << q_at
            };
            return base << hi_at | rest;
        }
        if self.compact {
            if item < s.n {
                let r = item as u64;
                let ones = ((1u64 << self.q_b) - 1) & !((1u64 << self.kb) - 1);
                let hi = s.r as u64 + (r >> self.k_i);
                let flags = if self.lean {
                    0
                } else {
                    1 << ob_at | u64::from(s.e[item] >= 0.0) << pos_at
                };
                return (ones | r) << q_at | hi << hi_at | flags;
            }
            let q = (item - s.n) as u64;
            return q << q_at | (q / s.c as u64) << hi_at;
        }
        if item < s.n {
            let r = item as u64;
            let hi = s.r as u64 + (r >> self.k_i);
            let flags = if self.lean {
                0
            } else {
                1 << ob_at | u64::from(s.e[item] >= 0.0) << pos_at
            };
            (r & mask) | hi << hi_at | ((1u64 << self.q_b) - 1) << q_at | flags
        } else {
            let q = (item - s.n) as u64;
            let r = q / s.c as u64;
            r << hi_at | q << q_at
        }
    }

    /// Outer bucket `i`'s word: `keep | own data | alt data`.
    #[must_use]
    pub fn outer_word(&self, i: u64) -> Word {
        let o = &self.map.outer;
        let (mu, d) = (o.mu as usize, self.outer_data_bits());
        let mut w = word(mu + 2 * d);
        let Ok(idx) = usize::try_from(i) else {
            return w;
        };
        if idx >= o.keep.len() {
            return w;
        }
        put(&mut w, 0, mu, u64::from(o.keep[idx]));
        if idx < self.spec.outer_items() {
            put(&mut w, mu, d, self.outer_data(idx));
        }
        put(&mut w, mu + d, d, self.outer_data(o.alt[idx] as usize));
        w
    }

    /// Bits of the inner word.
    #[must_use]
    pub fn inner_word_bits(&self) -> usize {
        self.map.inner[0].mu as usize + 2 * self.flag_bits() + self.k_i
    }

    /// Flag bits stored per inner item: `neg`, and `id` unless it is derived from `b`
    /// (`sa-toff`'s `derive_id`, `sa-pareto`'s lean layout).
    #[must_use]
    pub fn flag_bits(&self) -> usize {
        if self.derive_id || self.lean {
            1
        } else {
            2
        }
    }

    /// `(neg, id)` of inner item `j` of square `q` (`neg` only when `id` is derived).
    #[must_use]
    pub fn inner_flags(&self, q: usize, j: usize) -> u64 {
        let s = self.spec;
        if self.flag_bits() == 1 {
            return if j == s.b {
                u64::from(s.wb[q] < 0.0)
            } else {
                u64::from(s.w[q * s.b + j] > 0.0)
            };
        }
        if j == s.b {
            u64::from(s.wb[q] < 0.0) | 1 << 1
        } else {
            u64::from(s.w[q * s.b + j] > 0.0)
        }
    }

    /// The inner lookup's index range `R C 2^k_i`.
    #[must_use]
    pub fn inner_limit(&self) -> u64 {
        ((self.spec.r * self.spec.c) as u64) << self.k_i
    }

    /// Inner lookup word at `x = q 2^k_i + i`: `keep | own flags | alt b | alt flags`.
    #[must_use]
    pub fn inner_word(&self, x: u64) -> Word {
        let s = self.spec;
        let mut w = word(self.inner_word_bits());
        let q = usize::try_from(x >> self.k_i).unwrap_or(usize::MAX);
        if q >= s.r * s.c {
            return w;
        }
        let i = (x & ((1 << self.k_i) - 1)) as usize;
        let tab = &self.map.inner[s.n + q];
        let mu = tab.mu as usize;
        let f = self.flag_bits();
        put(&mut w, 0, mu, u64::from(tab.keep[i]));
        if i <= s.b {
            put(&mut w, mu, f, self.inner_flags(q, i));
        }
        let alt = tab.alt[i] as usize;
        put(&mut w, mu + f, self.k_i, alt as u64);
        put(&mut w, mu + f + self.k_i, f, self.inner_flags(q, alt));
        w
    }

    /// Number of network index values the angle lookup covers: `R 2^k_i + N`, or `E = R B + N`
    /// with the dense index.
    #[must_use]
    pub fn net_limit(&self) -> u64 {
        if self.dense {
            return (self.spec.r * self.spec.b + self.spec.n) as u64;
        }
        ((self.spec.r as u64) << self.k_i) + self.spec.n as u64
    }

    /// The network at index value `v`, if any.
    #[must_use]
    pub fn network(&self, v: u64) -> Option<&'a Network> {
        let s = self.spec;
        if self.dense {
            let v = usize::try_from(v).ok()?;
            return if v < s.r * s.b {
                Some(s.nets[v].as_ref())
            } else {
                s.e_nets.get(v - s.r * s.b).map(AsRef::as_ref)
            };
        }
        let hi = (v >> self.k_i) as usize;
        let lo = (v & ((1 << self.k_i) - 1)) as usize;
        if hi < s.r {
            (lo < s.b).then(|| s.nets[hi * s.b + lo].as_ref())
        } else {
            let r = (hi - s.r) << self.k_i | lo;
            (r < s.n).then(|| s.e_nets[r].as_ref())
        }
    }

    /// Qubits of the angle register: `sa-pareto`'s slots plus its flag bits (the sum of the
    /// rotation widths for every one-chunk layout).
    #[must_use]
    pub fn angle_bits(&self) -> usize {
        self.slot_bits() + self.angle_flags
    }

    /// Bits of `sa-pareto`'s slots alone.
    #[must_use]
    pub fn slot_bits(&self) -> usize {
        self.slot_w.iter().sum()
    }

    /// The slot layout for `chunks` chunks (`0` and `1` both mean one chunk; `sa-lowq`).
    #[must_use]
    pub fn slots(&self, chunks: usize) -> Slots {
        let r = self.modes.len();
        let g = r.div_ceil(chunks.clamp(1, r.max(1)));
        let chunks = r.div_ceil(g);
        let width: Vec<usize> = (0..g)
            .map(|s| (s..r).step_by(g).map(|j| self.widths[j]).max().unwrap_or(1))
            .collect();
        let at = starts(&width);
        Slots {
            g,
            chunks,
            width,
            at,
        }
    }

    /// Chunk `c`'s angles of index value `v` in the slot layout `sl` (`sa-lowq`).
    #[must_use]
    pub fn chunk_word(&self, v: u64, c: usize, sl: &Slots) -> Word {
        let mut w = word(sl.bits());
        if let Some(n) = self.network(v) {
            let r = n.rotations.len();
            for j in c * sl.g..((c + 1) * sl.g).min(r) {
                let s = j - c * sl.g;
                put(
                    &mut w,
                    sl.at[s],
                    sl.width[s],
                    stored_angle(self.spec, j, n.rotations[j].2),
                );
            }
        }
        w
    }

    /// Rotations of `sa-pareto`'s chunk `c`.
    #[must_use]
    pub fn chunk_rotations(&self, c: usize) -> std::ops::Range<usize> {
        c * self.g..((c + 1) * self.g).min(self.modes.len())
    }

    /// `sa-pareto`: the angle-register word of index value `v` while chunk `c` is loaded, with
    /// the flag bits (`is_ob | pos_e`, lean layout only) when `flags`.
    #[must_use]
    pub fn lean_word(&self, v: u64, c: usize, flags: bool) -> Word {
        let mut w = word(self.angle_bits());
        if let Some(n) = self.network(v) {
            for (slot, j) in self.chunk_rotations(c).enumerate() {
                let a = stored_angle(self.spec, j, n.rotations[j].2);
                put(&mut w, self.slot_at[slot], self.slot_w[slot], a);
            }
        }
        if flags && self.angle_flags > 0 {
            let s = self.spec;
            let hi = (v >> self.k_i) as usize;
            if hi >= s.r {
                let r = (hi - s.r) << self.k_i | (v & ((1 << self.k_i) - 1)) as usize;
                if r < s.n {
                    let f = 1 | u64::from(s.e[r] >= 0.0) << 1;
                    put(&mut w, self.slot_bits(), 2, f);
                }
            }
        }
        w
    }

    /// The angle word of index value `v`.
    #[must_use]
    pub fn angle_word(&self, v: u64) -> Word {
        let mut w = word(self.widths.iter().sum());
        if let Some(n) = self.network(v) {
            for (j, &(_, _, a)) in n.rotations.iter().enumerate() {
                put(
                    &mut w,
                    self.at[j],
                    self.widths[j],
                    stored_angle(self.spec, j, a),
                );
            }
        }
        w
    }

    /// Length of row `hv` of the ragged RPREP iteration: `B` for a square row (the identity
    /// item `b = B` is not a leaf), the one-body networks left for a one-body row.
    #[must_use]
    pub fn row_len(&self, hv: u64) -> u64 {
        let row = 1u64 << self.k_i;
        if (hv as usize) < self.spec.r {
            self.spec.b as u64
        } else {
            (self.net_limit() - hv * row).min(row)
        }
    }

    /// Rows of the ragged RPREP iteration.
    #[must_use]
    pub fn rows(&self) -> u64 {
        self.net_limit().div_ceil(1 << self.k_i)
    }

    /// The leaf `sa-pareto`'s lean ragged iteration visits for index value `v`: every lane's `v`
    /// is in range except a square's identity item `lo = B`, which a left-only node sends to the
    /// leaf `leaf_in_row(B)`. (Its angles then rotate and unrotate around an identity Majorana,
    /// so they do not matter; the erasure must use the same data, which this map gives it.) The
    /// legacy iteration visits `v` itself or nothing.
    #[must_use]
    pub fn leaf(&self, v: u64) -> u64 {
        if !self.lean {
            return v;
        }
        let row = 1u64 << self.k_i;
        let (hv, lo) = (v >> self.k_i, v & (row - 1));
        let hv = leaf_in_range(hv, self.h, self.rows());
        let lo = leaf_in_range(lo, self.k_i, self.row_len(hv));
        hv * row + lo
    }

    /// One colour class of the chain: every rotation's pair has exactly one mode in it.
    #[must_use]
    pub fn odd_class(&self) -> Vec<usize> {
        let top = self.modes.iter().map(|&(p, q)| p.max(q)).max().unwrap_or(0);
        let mut colour: Vec<Option<bool>> = vec![None; top + 1];
        loop {
            let mut changed = false;
            for &(p, q) in &self.modes {
                match (colour[p], colour[q]) {
                    (None, None) => {
                        colour[p] = Some(false);
                        colour[q] = Some(true);
                        changed = true;
                    }
                    (Some(c), None) => {
                        colour[q] = Some(!c);
                        changed = true;
                    }
                    (None, Some(c)) => {
                        colour[p] = Some(!c);
                        changed = true;
                    }
                    (Some(a), Some(c)) => assert_ne!(a, c, "mode graph is not bipartite"),
                }
            }
            if !changed {
                break;
            }
        }
        (0..=top).filter(|&m| colour[m] == Some(true)).collect()
    }
}

/// The value an in-range unary iteration over `nbits` index bits and `limit` values visits for
/// index value `v`: at a node with only a left child the index bit is not read (the caller
/// guarantees it is 0), so `v` loses every such bit.
#[must_use]
pub fn leaf_in_range(v: u64, nbits: usize, limit: u64) -> u64 {
    let mut base = 0u64;
    for bit in (0..nbits).rev() {
        let half = 1u64 << bit;
        if base + half < limit && v >> bit & 1 == 1 {
            base += half;
        }
    }
    base
}
