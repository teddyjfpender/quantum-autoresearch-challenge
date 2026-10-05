//! UNTRUSTED stage: runs the contestant's `walk::build` and writes `ops.bin`, `lanemap.bin` and
//! `family.out.json` into the current directory. Nothing is simulated or scored here; the
//! trusted `eval_circuit` re-reads these files in a separate process built without `walk`.
//!
//! The spec is read from `<root>/specs/<walk::spec_id()>/`, where `<root>` is `$FEMOCO_ROOT` or
//! the repository this binary was built from.
use femoco_walk::circuit::{write_ops, Builder};
use femoco_walk::score::FamilyOut;
use femoco_walk::{spec, walk};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let root = std::env::var_os("FEMOCO_ROOT")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let id = walk::spec_id();
    println!("-- spec {id} from {}", root.display());
    let spec = spec::load(&root, id)?;
    let mut b = Builder::new(spec.system_qubits());
    let lm = walk::build(spec.as_ref(), &mut b);
    if lm.uniform_bits() != b.uniform_bits() {
        return Err(format!(
            "lane map declares u = {} but the builder was given u = {}",
            lm.uniform_bits(),
            b.uniform_bits()
        ));
    }
    let ops = b.finish();
    println!("  emitted ops : {}", ops.len());
    write_ops(&ops, &PathBuf::from("ops.bin")).map_err(|e| format!("ops.bin: {e}"))?;
    std::fs::write("lanemap.bin", lm.to_bytes()).map_err(|e| format!("lanemap.bin: {e}"))?;
    let fam = FamilyOut {
        family: walk::family(),
        spec: id.to_string(),
    };
    let json = serde_json::to_vec_pretty(&fam).map_err(|e| e.to_string())?;
    std::fs::write("family.out.json", json).map_err(|e| format!("family.out.json: {e}"))?;
    println!("  wrote ops.bin, lanemap.bin, family.out.json");
    Ok(())
}

fn main() {
    println!("=== femoco_walk: build_circuit (untrusted stage) ===");
    if let Err(e) = run() {
        eprintln!("!! build_circuit: {e}");
        std::process::exit(2);
    }
}
