//! Lane-engine equivalence on every measured spectrum-amplification circuit (those pinned in
//! tests/sa_digests.rs, shared through tests/sa_circuits/list.rs): each is rebuilt, its digests
//! are checked against the pinned ones, and the candidate engine (`FEMOCO_EQUIV_ENGINE`) is
//! compared with the reference under every seed mode and thread count (tests/equiv_common).
//! Heavy (seconds to minutes per circuit): ignored by default, run by
//! tools/fastsim/equivalence.sh.
#![cfg(feature = "walk")]
mod equiv_common;

use femoco_walk::spec::sa::SaSpec;
use femoco_walk::taxonomy::Family;
use femoco_walk::walk::sa_low::{self, Params, Tweaks};

fn pinned(id: &str, name: &str, family: Family, p: impl Fn(&SaSpec) -> Params, want: [&str; 4]) {
    let a = equiv_common::artifact(name, id, family, |spec, b| {
        let sa: &SaSpec = spec.as_any().downcast_ref().unwrap();
        sa_low::build_with(sa, b, p(sa))
    });
    use sha2::{Digest, Sha256};
    assert_eq!(a.ops.ops.len().to_string(), want[0], "{name}: ops count");
    assert_eq!(hex::encode(a.ops.sha256), want[1], "{name}: ops digest");
    assert_eq!(
        hex::encode(Sha256::digest(&a.lanemap)),
        want[2],
        "{name}: lanemap digest"
    );
    assert_eq!(
        hex::encode(Sha256::digest(&a.family)),
        want[3],
        "{name}: family digest"
    );
    a.check();
}

macro_rules! circuit {
    ($test:ident, $spec:literal, $family:expr, $params:expr, $want:expr) => {
        #[test]
        #[ignore = "heavy: tools/fastsim/equivalence.sh"]
        fn $test() {
            pinned($spec, stringify!($test), $family, $params, $want);
        }
    };
}

include!("sa_circuits/list.rs");
