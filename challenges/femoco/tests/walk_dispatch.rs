//! Every architecture `walk::build` dispatches must also have a declared family in
//! `walk::family`. A merge once dropped `family`'s arm for `sparse-lean` while `build` kept it,
//! which would have panicked only when that architecture ran. Read the dispatcher's source so the
//! check covers every arm, including or-patterns.
use std::collections::BTreeSet;

fn arms(src: &str, func: &str) -> BTreeSet<(String, String)> {
    let start = src.find(func).expect("function present");
    let body = &src[start..src[start..].find("\n}\n").map_or(src.len(), |e| start + e)];
    let mut out = BTreeSet::new();
    for line in body.lines().map(str::trim).filter(|l| l.starts_with("(\"")) {
        let head = &line[1..line.find(')').expect("closing paren")];
        let (archs, enc) = head.rsplit_once(", ").expect("arch, encoding");
        let enc = enc
            .split_whitespace()
            .next()
            .unwrap_or(enc)
            .trim_matches('"')
            .to_string();
        for arch in archs.split('|') {
            out.insert((arch.trim().trim_matches('"').to_string(), enc.clone()));
        }
    }
    out
}

#[test]
fn every_built_architecture_declares_a_family() {
    let src = std::fs::read_to_string("src/walk/mod.rs").unwrap();
    let built = arms(&src, "pub fn build(");
    let declared = arms(&src, "pub fn family(");
    // family() dispatches on the spec id's encoding, where nested DF reads "df-nested" and the
    // sos-sa specs (`<instance>-sa-v1`) read "sa".
    let missing: Vec<_> = built
        .iter()
        .filter(|(arch, enc)| {
            let direct = declared.contains(&(arch.clone(), enc.clone()));
            let nested = enc == "df" && declared.contains(&(arch.clone(), "df-nested".into()));
            let sa = enc == "sos-sa" && declared.contains(&(arch.clone(), "sa".into()));
            !(direct || nested || sa)
        })
        .collect();
    assert!(!built.is_empty());
    assert!(missing.is_empty(), "built but no family: {missing:?}");
}
