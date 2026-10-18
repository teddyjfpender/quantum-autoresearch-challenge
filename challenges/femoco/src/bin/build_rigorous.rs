//! Local maintainer reproduction of the rigorous promotion catalogue. Not a judge or ledger
//! writer. The circuits are emitted by the same existing builders with runtime parameters;
//! a normal submission uses the catalogue's `build` knobs with `build_circuit` instead.
use femoco_walk::circuit::{write_ops, Builder};
use femoco_walk::score::FamilyOut;
use femoco_walk::spec::{self, sa::SaSpec};
use femoco_walk::walk::sa_low::{self, Params, Tweaks};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Catalogue {
    candidates: Vec<Candidate>,
}
#[derive(Deserialize)]
struct Candidate {
    id: String,
    track: String,
    spec: String,
    build: BTreeMap<String, String>,
}

fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: build_rigorous OUT_ROOT [CANDIDATE_ID]")?,
    );
    let id = args.next();
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let cat: Catalogue = serde_json::from_slice(
        &std::fs::read(root.join("rigorous/promotions.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut built = 0;
    for c in cat
        .candidates
        .into_iter()
        .filter(|c| id.as_ref().is_none_or(|id| id == &c.id))
    {
        if !slug(&c.id) || !slug(&c.track) {
            return Err("invalid candidate path".into());
        }
        let boxed = spec::load(root, &c.spec)?;
        let sa = boxed
            .as_any()
            .downcast_ref::<SaSpec>()
            .ok_or("not sos-sa")?;
        let mut p = Params::for_spec(sa);
        let get = |key: &str| {
            c.build
                .get(key)
                .ok_or_else(|| format!("missing knob {key}"))
        };
        let num = |key: &str| get(key)?.parse::<u32>().map_err(|e| e.to_string());
        p.outer.1 = num("FEMOCO_SA_MU_O")?;
        p.inner.1 = num("FEMOCO_SA_MU_I")?;
        p.outer_a = num("FEMOCO_SA_OUTER_A")? as usize;
        p.inner_a = num("FEMOCO_SA_INNER_A")? as usize;
        let family = match get("FEMOCO_WALK_ARCH")?.as_str() {
            "sa-toff" => {
                p.tw = Tweaks::parse(get("FEMOCO_SA_TWEAKS")?);
                sa_low::family_toff()
            }
            "sa-low2025" => {
                p.tw = Tweaks::OFF;
                sa_low::family(true)
            }
            arch => return Err(format!("unsupported promotion builder {arch}")),
        };
        let mut b = Builder::new(boxed.system_qubits());
        let map = sa_low::build_with(sa, &mut b, p);
        let ops = b.finish();
        let dir = out.join(&c.track).join(&c.id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        write_ops(&ops, &dir.join("ops.bin")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("lanemap.bin"), map.to_bytes()).map_err(|e| e.to_string())?;
        std::fs::write(
            dir.join("family.out.json"),
            serde_json::to_vec_pretty(&FamilyOut {
                family,
                spec: c.spec.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        println!(
            "{} / {}: {} ops -> {}",
            c.spec,
            c.id,
            ops.len(),
            dir.display()
        );
        built += 1;
    }
    if built == 0 {
        return Err("no candidate matched".into());
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("build_rigorous: {e}");
        std::process::exit(2);
    }
}
