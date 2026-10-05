// The measured spectrum-amplification circuits: parameter helpers and one `circuit!` line per
// circuit, `circuit!(test_name, "spec-id", family, params, [ops count, ops sha256, lanemap
// sha256, family sha256])`. Included by tests/sa_digests.rs (byte identity of each circuit) and
// tests/equiv_pinned.rs (each circuit under the reference and a candidate lane engine), which
// define `circuit!` and import `SaSpec`, `Params`, `Tweaks` and `sa_low` before the include.
/// `sa-low2025` (`swap`) and `sa-low2025-bothspin`: the published construction.
fn low(swap: bool) -> impl Fn(&SaSpec) -> Params {
    move |s| Params {
        swap,
        ..Params::for_spec(s)
    }
}

/// `sa-toff` with `FEMOCO_SA_TWEAKS = tw`.
fn toff(tw: &'static str) -> impl Fn(&SaSpec) -> Params {
    move |s| Params {
        tw: Tweaks::parse(tw),
        ..Params::for_spec(s)
    }
}

/// `sa-pareto` with `FEMOCO_SA_CHUNKS`, `_INNER_A`, `_OUTER_A`, `_CARRIES`, `_DROP_ALT` (lean on).
fn pareto(
    chunks: usize,
    inner_a: usize,
    outer_a: usize,
    carries: u32,
    drop_alt: bool,
) -> impl Fn(&SaSpec) -> Params {
    move |s| Params {
        pareto: true,
        lean: true,
        carries,
        chunks,
        drop_alt,
        inner_a,
        outer_a,
        ..Params::for_spec(s)
    }
}

/// `sa-toff` with `FEMOCO_SA_TWEAKS = tw` and keep widths `FEMOCO_SA_MU_O / _MU_I`.
fn toff_mu(tw: &'static str, mu_o: u32, mu_i: u32) -> impl Fn(&SaSpec) -> Params {
    move |s| {
        let p = toff(tw)(s);
        Params {
            outer: (p.outer.0, mu_o),
            inner: (p.inner.0, mu_i),
            ..p
        }
    }
}

/// `sa-toff` with `FEMOCO_SA_TWEAKS = tw` and `FEMOCO_SA_INNER_A = inner_a`.
fn toff_a(tw: &'static str, inner_a: usize) -> impl Fn(&SaSpec) -> Params {
    move |s| Params {
        inner_a,
        ..toff(tw)(s)
    }
}

// ---- The estimated rounding class (spec/SPEC-SA.md section 14; Low et al. 2025):
// `Params::for_spec` gives 9 + 9 keep bits on these specs. Label: "estimated rounding
// error (Low et al. class)"; not ranked with the rigorous class. ----

// E-low-R: sa-low2025 (SpinSwap), 9,600 / 1,040.
circuit!(
    est_low_r,
    "reiher-sa-est-v1",
    sa_low::family(true),
    low(true),
    [
        "1159447",
        "95e8fe8982dca129be65f4d11e4bdce0eb1fd4173f9294687ad0dba8646e2ea9",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "408bedae76909f608392dd78737b227c9273693caaf309e1123f0c94730d6f04",
    ]
);

// E-toff-R: sa-toff all, 9,364 / 1,028.
circuit!(
    est_toff_r,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff("all"),
    [
        "1176519",
        "58c5a23ae6877f9bf0b12a7d7199a6e2bc27556ef8e688ad5625e708a406f85d",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// E-toffL-R: sa-toff imchxL, 9,340 / 1,044.
circuit!(
    est_toffl_r,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff("imchxL"),
    [
        "1176237",
        "9644540e5173efa1497dd2498fd60b487866b733376b07021ad580e41a5ae092",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// E-toffL5-R: sa-toff imchxL inner_a 5, 9,312 / 1,044.
circuit!(
    est_toffl5_r,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_a("imchxL", 5),
    [
        "1290377",
        "e909e5bf97670a709ed6be1e01f1d07f96cfe56cedb1638a615e3228751b4e99",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// E-R1: sa-pareto C 1 ia 4 oa 2 carries 3, 9,459 / 1,050.
circuit!(
    est_r1,
    "reiher-sa-est-v1",
    sa_low::family_pareto(),
    pareto(1, 4, 2, 3, false),
    [
        "1160997",
        "b7000aa37e7a4e862c72f7afdac17cf1bb99b3fdbafb55a9130bdf85c49cd858",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "84aea03508570ad2ae4d7f183beb0df256896b4ebf15f54be572a20422473fde",
    ]
);

// E-R2: sa-pareto C 1 ia 4 oa 2 drop_alt, 9,525 / 1,016.
circuit!(
    est_r2,
    "reiher-sa-est-v1",
    sa_low::family_pareto(),
    pareto(1, 4, 2, 0, true),
    [
        "1165500",
        "bd05174b511c91e2eacc9d193ac4577452f7e169c42f694171defb9b07a4afc1",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "84aea03508570ad2ae4d7f183beb0df256896b4ebf15f54be572a20422473fde",
    ]
);

// E-Rs3: sa-pareto C 3 ia 4 oa 2 drop_alt, 12,101 / 494.
circuit!(
    est_rs3,
    "reiher-sa-est-v1",
    sa_low::family_pareto(),
    pareto(3, 4, 2, 0, true),
    [
        "1162417",
        "ea3ee0126013f55f5007152ffa0cea81b5b97357615ddec78e2de752d627b3ee",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "84aea03508570ad2ae4d7f183beb0df256896b4ebf15f54be572a20422473fde",
    ]
);

// The C = 1 one-hot levers on the estimated class (spec/SPEC-SA.md section 14),
// "estimated rounding error (Low et al. class)".
// E-R-lgr: toff("imchxlgr"), reiher-sa-est-v1, 9,298.003 / 573 (sampled mean).
circuit!(
    est_r_lgr,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff("imchxlgr"),
    [
        "1179277",
        "47b89d6d008e8f876e994633b8951c6fc3ed046131b023e5b9bf92fa8ccb7934",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// E-R-lgrp5: toff_a("imchxlgrp", 5), reiher-sa-est-v1, 9,206.005 / 682 (sampled mean).
circuit!(
    est_r_lgrp5,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_a("imchxlgrp", 5),
    [
        "3354503",
        "dd08e3c70ad7ba0725408fa35b0ea228177a195e4e140bb86acf2b0ced734d12",
        "3f112d8fad1dbe343c9db74c2a65a76c8bbc570fdbc950158acd5d52e3fbefe6",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// E-L-vd-a4: toff_a("imchxgrvd", 4), li-sa-est-v1, 18,205.000 / 745 (sampled mean).
circuit!(
    est_l_vd_a4,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_a("imchxgrvd", 4),
    [
        "4481474",
        "01888055ecc61f100bf3f8c1d7b5f252c13d0ba7f3b7808f7b06c26de4c84d71",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// E-L-gr-a5: toff_a("imchxgr", 5), li-sa-est-v1, "estimated rounding error (Low et al. class)",
// 13,340.993 / 1,219 (sampled mean).
circuit!(
    est_l_gr_a5,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_a("imchxgr", 5),
    [
        "3788410",
        "da11811ca8b5caf1aa223d12e3e5c8b1d642211c1c51ab03a8e67b6e601568ee",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

/// `sa-toff` with both keep widths, `FEMOCO_SA_INNER_A` (the paired lookup's slot width
/// with `G`) and `FEMOCO_SA_OUTER_A`.
fn toff_lw(
    tw: &'static str,
    mu: u32,
    inner_a: usize,
    outer_a: usize,
) -> impl Fn(&SaSpec) -> Params {
    move |s| {
        let p = toff_mu(tw, mu, mu)(s);
        Params {
            inner_a,
            outer_a,
            ..p
        }
    }
}

// lw-L4AF-m8: toff_lw("imchxgrdky4zabCAXZJSRNOBWMenF", 8, 8, 3), li-sa-est-v1, keep 8 + 8, full K, sliced engine
// (524,288 lanes) 18,055.421 / 482 (sampled mean).
circuit!(
    lw_l4af_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_lw("imchxgrdky4zabCAXZJSRNOBWMenF", 8, 8, 3),
    [
        "4538280",
        "222cb5f159a36562a547c3e7c13aa9a6b8bc25c8b4008bab531f3dc629cb9e49",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lw-L4AF-m9: toff_lw("imchxgrdky4zabCAXZJSRNOBWMenF", 9, 8, 3), li-sa-est-v1, keep 9 + 9, full K, sliced engine
// (524,288 lanes) 18,145.488 / 485 (sampled mean).
circuit!(
    lw_l4af_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_lw("imchxgrdky4zabCAXZJSRNOBWMenF", 9, 8, 3),
    [
        "4580124",
        "e4deb6e3b8ff2f668fe4a3502c2621d1b725a90c7132ffc72d578e593883ea85",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lw-L4BF-m8: toff_lw("imchxgrdky4zabCAXZJOBWMnF", 8, 8, 3), li-sa-est-v1, keep 8 + 8, full K, sliced engine
// (524,288 lanes) 17,903.475 / 488 (sampled mean).
circuit!(
    lw_l4bf_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_lw("imchxgrdky4zabCAXZJOBWMnF", 8, 8, 3),
    [
        "4536144",
        "c07e44836ee6a12bc32607e371fbd0f56bbfb576ca2ac8c763db71f357db0cd4",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lw-L4BF-m9: toff_lw("imchxgrdky4zabCAXZJOBWMnF", 9, 8, 3), li-sa-est-v1, keep 9 + 9, full K, sliced engine
// (524,288 lanes) 17,991.650 / 491 (sampled mean).
circuit!(
    lw_l4bf_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_lw("imchxgrdky4zabCAXZJOBWMnF", 9, 8, 3),
    [
        "4577964",
        "b705855dd5e11bafdf70cd33e0e1c519739df6aae096190d8eafccd5958cb22c",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lw-L4CF-m8: toff_lw("imchxgrdy4zabCAXZJOBMnF", 8, 8, 3), li-sa-est-v1, keep 8 + 8, full K, sliced engine
// (524,288 lanes) 17,836.496 / 497 (sampled mean).
circuit!(
    lw_l4cf_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_lw("imchxgrdy4zabCAXZJOBMnF", 8, 8, 3),
    [
        "4528495",
        "07b6c51dc3131da64bffdf27a5e8aa93995312e16777fbdf3061a7192da4dbe4",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// ---- Lever `s` (global sign normalisation of the Householder vectors) on the est-class
// fronts. Each passed the sliced engine at 524,288 lanes (`--engine sliced`) and the trusted
// evaluator at K = 64. ----

/// `sa-toff` with `FEMOCO_SA_TWEAKS = tw`, keep widths `(mu_o, mu_i)`, `INNER_A`, `OUTER_A`.
fn toff_mua(
    tw: &'static str,
    mu: (u32, u32),
    inner_a: usize,
    outer_a: usize,
) -> impl Fn(&SaSpec) -> Params {
    move |s| Params {
        inner_a,
        outer_a,
        ..toff_mu(tw, mu.0, mu.1)(s)
    }
}

// lr-L5s-m8: li-sa-est-v1, 20,367.509 / 436 (full K).
circuit!(
    lr_l5s_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky5zabCHVIXJs", (8, 8), 3, 2),
    [
        "5157076",
        "6ffde2edbacdaf61a571aab1b4d86fc50cfcef9bf77939faee6fb06a664d3ed7",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L5s-m9: li-sa-est-v1, 20,493.518 / 439 (full K).
circuit!(
    lr_l5s_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky5zabCHVIXJs", (9, 9), 3, 2),
    [
        "5212557",
        "df81c495984cc086c1b499fb4e6cad46782636425105ae7c1ebe27c482028423",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L4Zs-m8: li-sa-est-v1, 18,375.692 / 496 (full K).
circuit!(
    lr_l4zs_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJs", (8, 8), 3, 2),
    [
        "4925696",
        "63e4c09de14c7f3a4ced7bc36f17947f28fdb0e2af1e78a6c7184c8e41eac9a8",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L4Zs-m9: li-sa-est-v1, 18,501.579 / 499 (full K).
circuit!(
    lr_l4zs_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJs", (9, 9), 3, 2),
    [
        "4981177",
        "a35c986c2872c2b279aca8da958cf16e9e5b6eb6f6e14c05a7e2b5a7fa7c3e92",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-R3s-m8: reiher-sa-est-v1, 11,684.539 / 316 (full K).
circuit!(
    lr_r3s_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky3zabCHKVIXDs", (8, 8), 3, 2),
    [
        "1433532",
        "841b1ac929f9fcd282b40758d19ab42d2596f07041804680c16f0a7b91f16b5d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// lr-R3s-m9: reiher-sa-est-v1, 11,750.297 / 319 (full K).
circuit!(
    lr_r3s_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky3zabCHKVIXDs", (9, 9), 3, 2),
    [
        "1456805",
        "9b93e9571e9bc276ba9a5269d8e836202b14fff7b5387069498facb947d1cb34",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// lr-L1s-m9: li-sa-est-v1, 13,341.001 / 1,218 (full K; C = 1 corner).
circuit!(
    lr_l1s_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrs", (9, 9), 5, 2),
    [
        "3785868",
        "6d386519836dfee7bf5ad226a92d8e9b72b23289ffd0d3ab841ea7b8c01b4ebc",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-R1s-m9: reiher-sa-est-v1, 9,305.991 / 564 (full K; C = 1 corner).
circuit!(
    lr_r1s_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrs", (9, 9), 4, 2),
    [
        "1178023",
        "99db881847efc0c7481fdc8ab1ce0c256f3bdd0716320d4360dc04cffbef937c",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// Lever `o` (the class one-hot packed by length); full-K PASS (sliced engine).

// lr-l6so-m8: li-sa-est-v1, 22,489.735 / 413 (full K).
circuit!(
    lr_l6so_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky6zabCHVIXZJso", (8, 8), 3, 2),
    [
        "5262534",
        "bc0cf2de9375ee9922cbe7c20e9ced5177457bdd5eda9df40a5c438e1dfe08f4",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-l6so-m9: li-sa-est-v1, 22,615.434 / 416 (full K).
circuit!(
    lr_l6so_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky6zabCHVIXZJso", (9, 9), 3, 2),
    [
        "5318015",
        "4164f218df0bc5af840078a6c16bb170e8f88933e888f30db788ac7c05a3e729",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-l7so-m8: li-sa-est-v1, 24,927.453 / 387 (full K).
circuit!(
    lr_l7so_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky7zabCHVIXJso", (8, 8), 3, 2),
    [
        "5296540",
        "3afef3af9e0556349cb9a813186fc36bd5c188e8c6da4efc4e4eed90315d354a",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-l7so-m9: li-sa-est-v1, 25,053.444 / 390 (full K).
circuit!(
    lr_l7so_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky7zabCHVIXJso", (9, 9), 3, 2),
    [
        "5352021",
        "2825fcd3c57d86cab58df2424765765ccc12bdce6d91eb44851cfee11dc13872",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-l4zso-m8: li-sa-est-v1, 18,547.330 / 488 (full K).
circuit!(
    lr_l4zso_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJso", (8, 8), 3, 2),
    [
        "4947128",
        "2cb31f127f34c40ea6e6736858b51e63cacc4b2e4b08a081070b51d10c333ce0",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-l4zso-m9: li-sa-est-v1, 18,673.539 / 491 (full K).
circuit!(
    lr_l4zso_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJso", (9, 9), 3, 2),
    [
        "5002609",
        "dc06e799ca1a725962b8a1fd71be42b2d78d1ada8766f5ebe2a8f4c29b085b4b",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// OUTER_A = 3 on the G = 4 corner (calibration): the outer read's
// eight-block transient (457) stays under the SELECT plateau, -14 Toffolis. Full-K PASS.

// lr-L4Zs-m8-o3: li-sa-est-v1, 18,361.459 / 496 (full K).
circuit!(
    lr_l4zs_m8_o3,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJs", (8, 8), 3, 3),
    [
        "4927777",
        "f9ef351347b8e80a07bf764d0263c272d411d0dd3984fe3ea0c1ab2ca297e17b",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L4Zs-m9-o3: li-sa-est-v1, 18,491.664 / 499 (full K).
circuit!(
    lr_l4zs_m9_o3,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_mua("imchxgrdky4zabCHVIXZJs", (9, 9), 3, 3),
    [
        "4983581",
        "d16be416de14d6e8909e1f7bb247353c093ad5530dc1b5256007e9bee9f459cd",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

/// `sa-toff` with `FEMOCO_SA_TWEAKS = tw`, keep widths `(mu, mu)`, `FEMOCO_SA_INNER_A = ia`,
/// `FEMOCO_SA_OUTER_A = oa`.
fn toff_rp(tw: &'static str, mu: u32, ia: usize, oa: usize) -> impl Fn(&SaSpec) -> Params {
    move |s| {
        let p = Params::for_spec(s);
        Params {
            tw: Tweaks::parse(tw),
            outer: (p.outer.0, mu),
            inner: (p.inner.0, mu),
            inner_a: ia,
            outer_a: oa,
            ..p
        }
    }
}

// rp_r3p_m8: toff_rp("imchxgrdky3zabCHKVIXDSRNBWU", 8, 3, 2), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 11,666.566 / 314.
circuit!(
    rp_r3p_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIXDSRNBWU", 8, 3, 2),
    [
        "1420910",
        "8b3a4a57e8b63da3e60caf2a76165c05f31e3530a28e70ddb1ab992275f7dc4c",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// rp_r3y_m8: toff_rp("imchxgrdky3zabCHKVITXSRNOBW", 8, 2, 2), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 12,659.434 / 311.
circuit!(
    rp_r3y_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVITXSRNOBW", 8, 2, 2),
    [
        "1440651",
        "75b14ffb910a4887021b15588909fd312e50f02984d3453eab9040ee59074d12",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// rp_r4y_m8: toff_rp("imchxgrdky4zZabCHKVITXSRNOBW", 8, 2, 2), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 14,311.312 / 286.
circuit!(
    rp_r4y_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabCHKVITXSRNOBW", 8, 2, 2),
    [
        "1535311",
        "9f2b754ca5e5cc7e9a185e07f3595db7f9fce03728ad3a98f41a0a579de8b73d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// rp_r5y_m8: toff_rp("imchxgrdky5zabCHKVITXSRNOBW", 8, 2, 1), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 16,098.338 / 265.
circuit!(
    rp_r5y_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVITXSRNOBW", 8, 2, 1),
    [
        "1533672",
        "2851f6207cab38b221d6fc76ffa47c6cc14c9c8bfb354f51c6b4ef36c65106d3",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// rp_r6yf_m8: toff_rp("imchxgrdky6zZabfHKVITXSRNOBW", 8, 2, 1), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 17,938.591 / 256.
circuit!(
    rp_r6yf_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabfHKVITXSRNOBW", 8, 2, 1),
    [
        "1571608",
        "0540b4a56deb07acd6270319ca0b2d7b1db7444c34d7228b3deee59129f05c39",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// rp_r7yf_m8: toff_rp("imchxgrdky7zabfHKVITXSRNOBWU", 8, 2, 1), reiher-sa-est-v1, estimated class, keep 8 + 8;
// full K (sliced engine, 524,288 lanes) 19,312.578 / 253.
circuit!(
    rp_r7yf_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabfHKVITXSRNOBWU", 8, 2, 1),
    [
        "1556712",
        "7aaff619a512844acdc7b0736e6f619a35c8d1874878e585ca1c020e0235f11f",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ---- Compositions of the levers above. Each ops SHA-256 equals the one its
// full-K run recorded (sliced engine, 524,288 lanes; trusted K = 64 first).
// Knobs: `toff_rp(tweaks, mu, INNER_A, OUTER_A)`. ----

// lr-LW1sY, 18,043.497 / 482 (full K), keep 8 + 8.
circuit!(
    ltn_lw1sy_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJSRNOBWMenFsY", 8, 8, 3),
    [
        "4522620",
        "65e6341325ae21f9bef0f0f508304fcb95ff6519da9973083091681d3061dc5a",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-LW1sY, 18,133.572 / 485 (full K), keep 9 + 9.
circuit!(
    ltn_lw1sy_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJSRNOBWMenFsY", 9, 8, 3),
    [
        "4564464",
        "c3ac126abeb159ad83965d0da1f9063e65bd330e1eb1f622cc8174e74c7f8a92",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-LcBMsYU, 17,839.612 / 490 (full K), keep 8 + 8.
circuit!(
    ltn_lcbmsyu_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYU", 8, 8, 3),
    [
        "4477172",
        "093dcbecccc8148e90230ec6fee853c671209f0e833ec03c170fb6229596c0b2",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-LcBMsYU, 17,927.449 / 493 (full K), keep 9 + 9.
circuit!(
    ltn_lcbmsyu_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYU", 9, 8, 3),
    [
        "4518992",
        "41b7039081562b37c0d84b0fbe4aac0265f1aab256b01cf0a9726d910bf6017b",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-Lc3, 17,780.480 / 498 (full K), keep 8 + 8.
circuit!(
    ltn_lc3_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdy4zabCAXZJBMnFsYU", 8, 8, 3),
    [
        "4469587",
        "ae79536ea9200a596601f056dc75bcdf1ee8eb3a307d99b149c94304f964bf6a",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L5ts, 20,245.419 / 430 (full K), keep 8 + 8.
circuit!(
    ltn_l5ts_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWs", 8, 3, 2),
    [
        "5127386",
        "435cb63c1408c3b0008d87c84b9e19e4b9eac6c04d22ee62ee0e1975156a9cf1",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-L5ts, 20,373.232 / 433 (full K), keep 9 + 9.
circuit!(
    ltn_l5ts_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWs", 9, 3, 2),
    [
        "5183213",
        "01af7e12ab9f7f6b7a69d4c571fdc9fdee0e99229003b30e9157d39b35ccb713",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Item one-hot (`H`) read corner, 18,126.495 / 495 (full K), keep 8 + 8.
circuit!(
    ltn_ln4t_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdy4zabCAHVIXZJtBPYsU", 8, 3, 2),
    [
        "4860951",
        "aa67a2962029499d62076a23c1506d6a4aa94c2e4c9c8a93dd45d50166894041",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Item one-hot (`H`) read corner, 18,250.994 / 499 (full K), keep 9 + 9.
circuit!(
    ltn_ln4t_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdy4zabCAHVIXZJtBPYsU", 9, 3, 2),
    [
        "4915945",
        "bb07bc90d3f735629d7561bf27ecc0ab64a7df94df9b93c025f2107d49a43cff",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// lr-R3tNBWs, 11,610.477 / 313 (full K), keep 8 + 8.
circuit!(
    ltn_r3tnbws_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs", 8, 2, 2),
    [
        "1403236",
        "18837b0044795b1b0e3299a2da327ab23db1a4db2669ba82b80a8f94f7b8a0b0",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// lr-R3tNBWs, 11,676.491 / 316 (full K), keep 9 + 9.
circuit!(
    ltn_r3tnbws_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs", 9, 2, 2),
    [
        "1427931",
        "a7df1f1b65327d8de45f25c938cecc3f4d1cb2fd95589aa99cddc6e775f851e0",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ltn-R1lt, 8,832.386 / 528 (full K), keep 8 + 8.
circuit!(
    ltn_r1lt_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyEHKVIXt", 8, 3, 3),
    [
        "1467953",
        "4353216dbe8ca26ada673c9e006d0cda09827ebcecadb092ca7893e51642f05b",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ltn-R1lt, 8,901.546 / 532 (full K), keep 9 + 9.
circuit!(
    ltn_r1lt_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyEHKVIXt", 9, 3, 3),
    [
        "1492820",
        "f8e3e307fb21df938152535b3186b3de2f9f64841f5e9db8dcd7a7731439c0dc",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ltn-R1t, 8,865.466 / 518 (full K), keep 8 + 8.
circuit!(
    ltn_r1t_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyEHKVIXtRNOBW", 8, 3, 3),
    [
        "1468547",
        "f184ebaedcbe079659ae4f0aa396e912e1ca88551546af62444f79f278b6ccf5",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ltn-R1t, 8,937.523 / 521 (full K), keep 9 + 9.
circuit!(
    ltn_r1t_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyEHKVIXtRNOBW", 9, 3, 3),
    [
        "1493468",
        "1d2c06663bf40f0f7e6c4b8b4f8c32b2748330a76046ba10ba3ae73d7b1ccda1",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ---- Lever `q<b>`, rank-scheduled delivery, on Reiher (and Li G = 3) --------------------------
// Each ops SHA-256 equals its full-K run's (sliced engine, 524,288 lanes, eval OK).
// rdr-R3q15-m9: 11,638.208 / 318 (full K), keep 9 + 9.
circuit!(
    rdr_r3q15_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq15", 9, 2, 2),
    [
        "1876521",
        "d2d6a3f5df6234909bf0d44556965e0708a13b75ddb77318248680a72bc47543",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q15-m8: 11,572.423 / 315 (full K), keep 8 + 8.
circuit!(
    rdr_r3q15_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq15", 8, 2, 2),
    [
        "1851826",
        "ec86d4f8fbfece4358379bd599ca2a410ac136b333ee2fd094ef7e7d1dcb891d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q45-m9: 11,578.645 / 346 (full K), keep 9 + 9.
circuit!(
    rdr_r3q45_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq45", 9, 2, 2),
    [
        "1871767",
        "123a72ef9dfa6442d026b7263c9f180c571cdc0d916b22af462af3af6c74fe98",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q45-m8: 11,512.554 / 343 (full K), keep 8 + 8.
circuit!(
    rdr_r3q45_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq45", 8, 2, 2),
    [
        "1847072",
        "2d19932b3f58209bdf901580cf6e2a30e2df23995d3f8f4a10e3b10f80c955f9",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q75-m9: 11,514.466 / 376 (full K), keep 9 + 9.
circuit!(
    rdr_r3q75_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq75", 9, 2, 2),
    [
        "1878737",
        "31fad4c036553e9c2c2bacb7e7e5bc57a0c25038087663d4d969e4697f46a9e2",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q75-m8: 11,448.315 / 373 (full K), keep 8 + 8.
circuit!(
    rdr_r3q75_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq75", 8, 2, 2),
    [
        "1854042",
        "336c96539aba0740cd4497496a43a6e35527a22c06616b48c7dc24dff10bd941",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q90-m9: 11,484.602 / 391 (full K), keep 9 + 9.
circuit!(
    rdr_r3q90_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq90", 9, 2, 2),
    [
        "1874649",
        "2297275038077a7287b8561b92d0460b8879e8d95a85b2c05aecd6b34aaa0ba8",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q90-m8: 11,418.338 / 388 (full K), keep 8 + 8.
circuit!(
    rdr_r3q90_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq90", 8, 2, 2),
    [
        "1849954",
        "7934a634bba8ff5a6e724b9f533436ac14b09ae579a5e992a440eff0dd30ea6b",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q105-m9: 11,416.504 / 408 (full K), keep 9 + 9.
circuit!(
    rdr_r3q105_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq105", 9, 3, 3),
    [
        "2086990",
        "7f81def91bbf293c09b923e6b8fd9a25b0e22daa0cf1f9f9ef8bb3d5b3c09f20",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q105-m8: 11,346.570 / 404 (full K), keep 8 + 8.
circuit!(
    rdr_r3q105_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq105", 8, 3, 3),
    [
        "2062099",
        "7c43f4d597207c29e3e94834232e1e367597fff517dc4a4ac61a20f1d7632ef8",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q120-m9: 10,822.618 / 421 (full K), keep 9 + 9.
circuit!(
    rdr_r3q120_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq120", 9, 3, 3),
    [
        "1947128",
        "86f4144b3bf9a6abdc42adffef925abfce5df8367ce19b3722fb0876993a0d1d",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q120-m8: 10,752.580 / 418 (full K), keep 8 + 8.
circuit!(
    rdr_r3q120_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq120", 8, 3, 3),
    [
        "1922237",
        "6a7e87e469e3c1a8da4e56cf7e665da83fa7f023ea87f622a34b00ee4df374ce",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q135-m9: 10,258.411 / 436 (full K), keep 9 + 9.
circuit!(
    rdr_r3q135_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq135", 9, 3, 3),
    [
        "1806732",
        "573a2d04d7cdbd4f47a6f88570fd86b05473cab097f2d5bbdb8e637c48797546",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q135-m8: 10,188.498 / 433 (full K), keep 8 + 8.
circuit!(
    rdr_r3q135_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq135", 8, 3, 3),
    [
        "1781841",
        "c35f046eefed3e483f049e1508f79999f84fdfc5f6df9f494ff67a55200f7cf6",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q150-m9: 9,922.530 / 451 (full K), keep 9 + 9.
circuit!(
    rdr_r3q150_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq150", 9, 3, 3),
    [
        "1739716",
        "b66107883c08b6c280204a525f9257110b8a1c0d17f58e2909b44217189e0bcb",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q150-m8: 9,852.529 / 448 (full K), keep 8 + 8.
circuit!(
    rdr_r3q150_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq150", 8, 3, 3),
    [
        "1714825",
        "47ed974dfb9cfafbbbc86bc5e143ef2f884f6b52aadd93a7d8f04f292d2b00fa",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q165-m9: 9,538.567 / 466 (full K), keep 9 + 9.
circuit!(
    rdr_r3q165_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq165", 9, 3, 3),
    [
        "1616444",
        "4b032ebb2baa2d18da33dd8152b75ea61569f60e0c63e91a63a31cc5a8e33964",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q165-m8: 9,468.448 / 463 (full K), keep 8 + 8.
circuit!(
    rdr_r3q165_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq165", 8, 3, 3),
    [
        "1591553",
        "daf9a9486a148dfd6f1a7e18cfca51d614eccb7aca6580589b9fb655b72893f7",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q180-m9: 9,330.491 / 481 (full K), keep 9 + 9.
circuit!(
    rdr_r3q180_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq180", 9, 3, 3),
    [
        "1562478",
        "0b613e701bd3a8059c75910f74b4fa255099a93c6acb5639c0df48df96db8fe8",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q180-m8: 9,260.325 / 478 (full K), keep 8 + 8.
circuit!(
    rdr_r3q180_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq180", 8, 3, 3),
    [
        "1537587",
        "fd8258209aa198e0d866218db29eeb123ae4c8ffb8550a42560cf9dd4b3cc413",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q195-m9: 9,116.476 / 496 (full K), keep 9 + 9.
circuit!(
    rdr_r3q195_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq195", 9, 3, 3),
    [
        "1566174",
        "b66a1f61af5602502039594bde49b1634d3df602dbffd504dea058d46954235c",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q195-m8: 9,046.565 / 493 (full K), keep 8 + 8.
circuit!(
    rdr_r3q195_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq195", 8, 3, 3),
    [
        "1541283",
        "31e51046a6daf795d24c0594a9b3736d2c9e4b46736165b7f05b0949daab7272",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q210-m9: 8,902.437 / 516 (full K), keep 9 + 9.
circuit!(
    rdr_r3q210_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq210", 9, 3, 3),
    [
        "1164082",
        "d6cb25ff150e8da9b004869ed5b686a9241bb3404f8c4fc8fb61c47a3caa75f4",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q210-m8: 8,832.596 / 513 (full K), keep 8 + 8.
circuit!(
    rdr_r3q210_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq210", 8, 3, 3),
    [
        "1139191",
        "09e0cd16153783a0606ff68bcea8e56127d12031b00216a72b0373e2b014bb4a",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3lq210-m9: 8,894.334 / 524 (full K), keep 9 + 9.
circuit!(
    rdr_r3lq210_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdky3zabCHKVIDXtNBWsq210", 9, 3, 3),
    [
        "1163874",
        "2ce695f251679c941d95bcb8723d57228f02b8284c73899e3d5fcc98a0b5f795",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3lq210-m8: 8,825.520 / 520 (full K), keep 8 + 8.
circuit!(
    rdr_r3lq210_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdky3zabCHKVIDXtNBWsq210", 8, 3, 3),
    [
        "1139011",
        "5f5475006815540b1a6f4f6e9f7b297fbb4d94a7b79e413df093f8831032ecc6",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-L3-m9: 16,493.206 / 555 (full K), keep 9 + 9.
circuit!(
    rdr_l3_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWs", 9, 3, 2),
    [
        "4853233",
        "ec26afaec23a8f51e897cddbf27009b5fc4208e242a6e1c1693fe1d5000bb6ec",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdr-L3q320-m9: 15,869.360 / 855 (full K), keep 9 + 9.
circuit!(
    rdr_l3q320_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWsq320", 9, 3, 2),
    [
        "6976455",
        "ba0cf2e15194dd0a44440379b2388439968be7e08ba6fda18b9575ef62b4f683",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdr-L3q400-m9: 14,885.479 / 939 (full K), keep 9 + 9.
circuit!(
    rdr_l3q400_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWsq400", 9, 3, 2),
    [
        "6258743",
        "c17237463c42e45ea890478c0bf8410b7269891a615234129df1467353f8e669",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdr-L3q480-m9: 14,205.445 / 1024 (full K), keep 9 + 9.
circuit!(
    rdr_l3q480_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWsq480", 9, 3, 2),
    [
        "5936255",
        "6ee546fd884a40ab9439cb235ed9fca0107fa0586a1c98fedfe0b343aa6ada5c",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdr-L3q520-m9: 13,785.537 / 1066 (full K), keep 9 + 9.
circuit!(
    rdr_l3q520_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWsq520", 9, 3, 2),
    [
        "5800263",
        "1d9e3143645aad90edcacb096ca07ce5edd48e5825d65cc1886958c622608c7a",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdr-L3q640-m9: 13,529.485 / 1167 (full K), keep 9 + 9.
circuit!(
    rdr_l3q640_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHVIXJtRNOBWsq640", 9, 3, 2),
    [
        "4557313",
        "6c0d9552f37f123d841be8b03dc3a97519318194dbe9a827fdb3101d51df8157",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Lever `q<b>_1` (furthest-future keep plan).
// rdr-R3q20b-m9: 11,552.462 / 323 (full K), keep 9 + 9.
circuit!(
    rdr_r3q20b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq20_1", 9, 2, 2),
    [
        "1812923",
        "5988bef6addabd21127e81ed04b947f953d6dada549649f1b45eff0c896bac8f",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q30b-m9: 11,412.537 / 333 (full K), keep 9 + 9.
circuit!(
    rdr_r3q30b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq30_1", 9, 2, 2),
    [
        "1774279",
        "59dee5c58b8dedb58dda60a74753584538391f9c3440a7871d05a0c26363ec59",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q45b-m9: 11,202.541 / 348 (full K), keep 9 + 9.
circuit!(
    rdr_r3q45b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq45_1", 9, 2, 2),
    [
        "1713429",
        "5128006207aae5f0745f98edd37e595401b6cc276b205a2ed5cc1e16203f58a1",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q60b-m9: 10,992.283 / 363 (full K), keep 9 + 9.
circuit!(
    rdr_r3q60b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq60_1", 9, 2, 2),
    [
        "1651451",
        "099fe3530c06cf1a34c32e8b08454841ecfd92a3a9d05c7fe856e6c410cb5115",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q75b-m9: 10,782.657 / 378 (full K), keep 9 + 9.
circuit!(
    rdr_r3q75b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq75_1", 9, 2, 2),
    [
        "1591951",
        "900c786fa57d21d4aab914f923dd311fcd33ceb8d287a28dc55960fef9aeeabf",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q90b-m9: 10,572.251 / 393 (full K), keep 9 + 9.
circuit!(
    rdr_r3q90b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq90_1", 9, 2, 2),
    [
        "1533655",
        "bec86e540a0e4d393a87aeb4ba805e928089f67d40e16e666d213cbce22dc4a5",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q105b-m9: 10,348.532 / 408 (full K), keep 9 + 9.
circuit!(
    rdr_r3q105b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq105_1", 9, 3, 3),
    [
        "1471554",
        "f8fea0534241826b458491d0ccf4426008a0f2fac3b8cc2bf4d1160272465aca",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q120b-m9: 10,138.630 / 423 (full K), keep 9 + 9.
circuit!(
    rdr_r3q120b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq120_1", 9, 3, 3),
    [
        "1413638",
        "54e51a1d7d71390f2d79f7b0478439a41e4f4b0322d587d7ac52be84784e1408",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q135b-m9: 9,928.641 / 438 (full K), keep 9 + 9.
circuit!(
    rdr_r3q135b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq135_1", 9, 3, 3),
    [
        "1359764",
        "c505b0806e3074fb14d92c834589c4b4669a374722c46dc9a3d4322a884f7f99",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q150b-m9: 9,718.406 / 453 (full K), keep 9 + 9.
circuit!(
    rdr_r3q150b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq150_1", 9, 3, 3),
    [
        "1306650",
        "7357965ea8e4dbc0cbaaf238a76f819b7d1687150928659af22ac85c026c906f",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q165b-m9: 9,508.489 / 468 (full K), keep 9 + 9.
circuit!(
    rdr_r3q165b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq165_1", 9, 3, 3),
    [
        "1253674",
        "e3ae606cd93f3ab72d2395b0117b44c42eaf10938c3a2a60cf6dbcdc97c480b6",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q180b-m9: 9,298.370 / 483 (full K), keep 9 + 9.
circuit!(
    rdr_r3q180b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq180_1", 9, 3, 3),
    [
        "1203314",
        "c71970cb70bd510816840cc0ed6505df100e8d575db03ea856c8242f7083847e",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q195b-m9: 9,088.351 / 498 (full K), keep 9 + 9.
circuit!(
    rdr_r3q195b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq195_1", 9, 3, 3),
    [
        "1137300",
        "b374e06c1f4f26567bec56407117851cf6a0986e80e3d78a4cdddae95ceebf1e",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q210b-m9: 8,902.490 / 513 (full K), keep 9 + 9.
circuit!(
    rdr_r3q210b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq210_1", 9, 3, 3),
    [
        "1028514",
        "3f4d8995ac00d327b2f38df75f6c62f1bdec9c5d272cbb202939ef4c6e2da2aa",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3lq210b-m9: 8,894.486 / 521 (full K), keep 9 + 9.
circuit!(
    rdr_r3lq210b_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdky3zabCHKVIDXtNBWsq210_1", 9, 3, 3),
    [
        "1028306",
        "228b56e4107aa31bf2eca01e1d4987f3d657682596582148de6a907529516230",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q20b-m8: 11,486.527 / 320 (full K), keep 8 + 8.
circuit!(
    rdr_r3q20b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq20_1", 8, 2, 2),
    [
        "1788228",
        "cc3da56bd87e2a729d1f8af2fa1d0a1fe0f32284de36d1929cfe10d9ea9965fc",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q30b-m8: 11,346.468 / 330 (full K), keep 8 + 8.
circuit!(
    rdr_r3q30b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq30_1", 8, 2, 2),
    [
        "1749584",
        "6c9845fd565418adbfad2f4671fff37ca2f4f89406feb53e57ec3f0da7744c41",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q45b-m8: 11,136.545 / 345 (full K), keep 8 + 8.
circuit!(
    rdr_r3q45b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq45_1", 8, 2, 2),
    [
        "1688734",
        "9d68b9d845009d2c649c54aafe71345ddd781c31f0aa97edb5c2def630ac2781",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q60b-m8: 10,926.291 / 360 (full K), keep 8 + 8.
circuit!(
    rdr_r3q60b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq60_1", 8, 2, 2),
    [
        "1626756",
        "7fbead6da36a0457ff9b2a30992c71119011fe06eba7c026c11b4694afcd73ef",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q75b-m8: 10,716.714 / 375 (full K), keep 8 + 8.
circuit!(
    rdr_r3q75b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq75_1", 8, 2, 2),
    [
        "1567256",
        "1b6d6345f82a15b553e914090bd4f7fa499fb5507b7170c83fda8365497b5bd3",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q90b-m8: 10,506.387 / 390 (full K), keep 8 + 8.
circuit!(
    rdr_r3q90b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq90_1", 8, 2, 2),
    [
        "1508960",
        "fb4425885ddba611b0dd092feedb66a93d7e86813aef931766b80821de89b07d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q105b-m8: 10,278.465 / 405 (full K), keep 8 + 8.
circuit!(
    rdr_r3q105b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq105_1", 8, 3, 3),
    [
        "1446663",
        "fa88d64e43d106f5a5a424450137971507b8519f9062132f1a1d707a92d9bce6",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q120b-m8: 10,068.500 / 420 (full K), keep 8 + 8.
circuit!(
    rdr_r3q120b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq120_1", 8, 3, 3),
    [
        "1388747",
        "b89b5640bb3db50bd562c4b6d4c1ad451b0d4d2c408692774f62605fb38f1145",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q135b-m8: 9,858.556 / 435 (full K), keep 8 + 8.
circuit!(
    rdr_r3q135b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq135_1", 8, 3, 3),
    [
        "1334873",
        "df05936e0a3a1d187beeeffc8362dea88cea47a845cd1704186cedde483407d0",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q150b-m8: 9,648.442 / 450 (full K), keep 8 + 8.
circuit!(
    rdr_r3q150b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq150_1", 8, 3, 3),
    [
        "1281759",
        "1cf9ad654db0b3c386e448a5298810d5229c4a368d7d973ae48b56c423b39367",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q165b-m8: 9,438.619 / 465 (full K), keep 8 + 8.
circuit!(
    rdr_r3q165b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq165_1", 8, 3, 3),
    [
        "1228783",
        "ab9d6a4e3fea247c7ab54988f13d8e1126bf27f47991029dc79e5ace0a1b2e0d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q180b-m8: 9,228.511 / 480 (full K), keep 8 + 8.
circuit!(
    rdr_r3q180b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq180_1", 8, 3, 3),
    [
        "1178423",
        "b27345a16a426777236f1c01aa27519c233c5236d34d4ef0f53ff0c17a431d65",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q195b-m8: 9,018.534 / 495 (full K), keep 8 + 8.
circuit!(
    rdr_r3q195b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq195_1", 8, 3, 3),
    [
        "1112409",
        "4fc922f27d59e4f8661e7eed895af95ff294518f14a2f5c1b8466743ee5ed1a7",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3q210b-m8: 8,832.519 / 510 (full K), keep 8 + 8.
circuit!(
    rdr_r3q210b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWsq210_1", 8, 3, 3),
    [
        "1003623",
        "f223e4a969ea80a62bc6372505d7786372b681722408a6f850e5b0fdb305a31f",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rdr-R3lq210b-m8: 8,825.634 / 517 (full K), keep 8 + 8.
circuit!(
    rdr_r3lq210b_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdky3zabCHKVIDXtNBWsq210_1", 8, 3, 3),
    [
        "1003443",
        "7cb4eec766c2069b3288f4a24802d799000c83659a51109df6b7bd26974b5b9d",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ---- Lever `q<b>_1` on Li, G = 3 .. 7 -----------------------------------------------------------
// Each ops SHA-256 equals its full-K run's (sliced engine, 524,288 lanes, eval OK).
// rdl-L7q22-m9: 24,825.248 / 400 (full K), keep 9 + 9.
circuit!(
    rdl_l7q22_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHVIXJsoq22_1", 9, 3, 2),
    [
        "6762909",
        "ac194c5bbeef0435b4dc8ab9417b7e3ed76c43ef30c5fcbfddbdcc607deb05ce",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L6q16-m9: 22,515.523 / 420 (full K), keep 9 + 9.
circuit!(
    rdl_l6q16_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsoq16_1", 9, 3, 2),
    [
        "6718979",
        "cae5103ad84e458c6a0ad726c73a09a18f1f4cb6731872ce2ae6583591e22884",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L6q26-m9: 22,367.579 / 430 (full K), keep 9 + 9.
circuit!(
    rdl_l6q26_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsoq26_1", 9, 3, 2),
    [
        "6683605",
        "7f7c3bfb83c6320c23003b7aa80622e6d0932278c489b26809c189e47665a5e5",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q27-m9: 20,161.557 / 450 (full K), keep 9 + 9.
circuit!(
    rdl_l5q27_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq27_1", 9, 3, 2),
    [
        "6568559",
        "5dfb77c2ab9c8d8b4dab039561e45788fa4d71ff5374f47a795fa3f502b19b1e",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q37-m9: 20,041.681 / 460 (full K), keep 9 + 9.
circuit!(
    rdl_l5q37_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq37_1", 9, 3, 2),
    [
        "6533337",
        "f55595ade47dd2bb9ee7f9b815bacca35f0a8e87d19330fc9bc84bf0e846c56a",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q47-m9: 19,921.590 / 470 (full K), keep 9 + 9.
circuit!(
    rdl_l5q47_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq47_1", 9, 3, 2),
    [
        "6489149",
        "527f777275bcc9979ded62c34a7e37f973138715394f3b3e513c18b0edc55187",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q57-m9: 19,801.525 / 480 (full K), keep 9 + 9.
circuit!(
    rdl_l5q57_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq57_1", 9, 3, 2),
    [
        "6434251",
        "b381aa78e9e421bc0af9aed62eb78ede31f7dd143c6f4506f610234628b997a6",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L4q21-m9: 17,867.469 / 499 (full K), keep 9 + 9.
circuit!(
    rdl_l4q21_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUq21_1", 9, 8, 3),
    [
        "5975192",
        "140b62dba07393fce172d9f86f112171e39a3c9a243b6e569b5cab89e69ff45a",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L4q42-m9: 17,679.570 / 520 (full K), keep 9 + 9.
circuit!(
    rdl_l4q42_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUq42_1", 9, 8, 3),
    [
        "5873862",
        "39b5ebef7d9415592e60785d49de3b8a2d5b90e6fb73e46307b10a52fe6f942d",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L4q62-m9: 17,499.613 / 540 (full K), keep 9 + 9.
circuit!(
    rdl_l4q62_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUq62_1", 9, 8, 3),
    [
        "5789376",
        "9f890b9661bb9f0b23aedff8b71797a2d919d20d90fd016cc4e0e4902e7a8bc0",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3s-m9: 16,217.555 / 555 (full K), keep 9 + 9.
circuit!(
    rdl_l3s_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJSRNOBWMenFs", 9, 8, 3),
    [
        "4452170",
        "1781cbf4292e67c37454255cefdbc8c37ea4ce9a5bf9c77e0a6d5c41d866b7ec",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q50-m9: 15,809.353 / 600 (full K), keep 9 + 9.
circuit!(
    rdl_l3q50_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq50_1", 9, 8, 3),
    [
        "5823330",
        "aafa9df08327acc845fca1c1d7ced23bf7313448ef775c62e1e6d74e3d5b1ad1",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q100-m9: 15,509.511 / 650 (full K), keep 9 + 9.
circuit!(
    rdl_l3q100_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq100_1", 9, 8, 3),
    [
        "5561766",
        "667cd5d38c4a758abca6479f12cf070caa7f6fd949f2521d0ae8d290d4918849",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q150-m9: 15,209.432 / 700 (full K), keep 9 + 9.
circuit!(
    rdl_l3q150_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq150_1", 9, 8, 3),
    [
        "5301404",
        "2b1fcc649fbdd957d33a6a77e9fe3e95d7f99e050067e744ff5c3901bc44cec2",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q250-m9: 14,609.517 / 800 (full K), keep 9 + 9.
circuit!(
    rdl_l3q250_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq250_1", 9, 8, 3),
    [
        "4759942",
        "db4d417fa7e6251324b45df3ed3ba45d8084c22a905e49206c4b703ee76ace2f",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q350-m9: 14,009.576 / 900 (full K), keep 9 + 9.
circuit!(
    rdl_l3q350_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq350_1", 9, 8, 3),
    [
        "4356490",
        "ceb4b158c4cbe879d6e4a10a0b8d3c5e9c17eb8cc5a3cd3a22fcda4063e36e98",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q450-m9: 13,425.413 / 1000 (full K), keep 9 + 9.
circuit!(
    rdl_l3q450_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq450_1", 9, 8, 3),
    [
        "3591036",
        "c0cb4fcac38756543fb03d3cd3ec3fea99c67d7e23e177066c0970701513e472",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q550-m9: 13,225.345 / 1100 (full K), keep 9 + 9.
circuit!(
    rdl_l3q550_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq550_1", 9, 8, 3),
    [
        "3368422",
        "b41959beb51e27d2690056e30dfec652985348ede587a31fd79c3fab4c3d7460",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q618-m9: 13,089.416 / 1168 (full K), keep 9 + 9.
circuit!(
    rdl_l3q618_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq618_1", 9, 8, 3),
    [
        "3164966",
        "e3161bdfc2a5cc9c2335362fc2e8d905989d537a875811ad0cf5754b588c3b33",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L6q29-m8: 22,197.399 / 430 (full K), keep 8 + 8.
circuit!(
    rdl_l6q29_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsoq29_1", 8, 3, 2),
    [
        "6616484",
        "157f47c3e1362e80914bdb0285d2a97e8b00bdaf0f4be7fd9d8db336f16c91a8",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q30-m8: 19,997.615 / 450 (full K), keep 8 + 8.
circuit!(
    rdl_l5q30_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq30_1", 8, 3, 2),
    [
        "6508334",
        "56adc90e01d3fe5870a138590ce164f0c5a066861a26b92ec623a54fd055e79d",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L5q60-m8: 19,637.420 / 480 (full K), keep 8 + 8.
circuit!(
    rdl_l5q60_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq60_1", 8, 3, 2),
    [
        "6361112",
        "bdd7145db92a4519bd38a9a73d6423c3d92e1b68988bd22c759ed854432909da",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L4q24-m8: 17,753.720 / 499 (full K), keep 8 + 8.
circuit!(
    rdl_l4q24_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUq24_1", 8, 8, 3),
    [
        "5916524",
        "e043787f45304c95f0f136d398c2941515fe0a64d5678c93f7c38c0a09bc90bd",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q250-m8: 14,521.585 / 797 (full K), keep 8 + 8.
circuit!(
    rdl_l3q250_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq250_1", 8, 8, 3),
    [
        "4718122",
        "7e4e3dc19a63d771da9491b687412fb610a3ed15e4a332c4adb7cba302d7901e",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rdl-L3q618-m8: 13,001.527 / 1165 (full K), keep 8 + 8.
circuit!(
    rdl_l3q618_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUq618_1", 8, 8, 3),
    [
        "3123146",
        "e2c1f1ca9bf6e2dc205dc0cd9537ce84fb2b3f776bc7849262664c069bdf726d",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// ---- Levers `+` (donor-padded inner tables, item-constant offset) and `t` on the paired
// lookup. Each ops digest equals its full-K run's (sliced engine, 524,288 lanes, eval OK, after
// the trusted K = 64 check).

// pf_r3p_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,652.678 / 316.
circuit!(
    pf_r3p_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+", 9, 2, 2),
    [
        "1367093",
        "db93b462938a630a2507121f9997b086f7494de6d8faeafbbbb278cf314fb361",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r3p_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,586.476 / 313.
circuit!(
    pf_r3p_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+", 8, 2, 2),
    [
        "1342424",
        "c61759501b0dbe0ec2685386238946d21b2bd1e97e2597a768860a7c30b23868",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r1p_m9: toff_rp("imchxlgrdkyEHKVIXt+", 9, 3, 3), reiher-sa-est-v1, keep 9 + 9; full-K 8,877.495 / 532.
circuit!(
    pf_r1p_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyEHKVIXt+", 9, 3, 3),
    [
        "1431982",
        "a194ec9ffdf331cc992f1bbff87784428fdd63853ea1e9781059c2b5b9ef0e86",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r1p_m8: toff_rp("imchxlgrdkyEHKVIXt+", 8, 3, 3), reiher-sa-est-v1, keep 8 + 8; full-K 8,808.535 / 528.
circuit!(
    pf_r1p_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyEHKVIXt+", 8, 3, 3),
    [
        "1407141",
        "2c10be0cee1b0e4649445d16c36e82f2e3bfdac01d1f46a180199e32fcb0a6e6",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_l4t_m9: toff_rp("imchxgrdky4zabCAXZJBMnFsYUt", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,855.487 / 492.
circuit!(
    pf_l4t_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUt", 9, 8, 3),
    [
        "4480294",
        "064126088929e5c9064d782317c7ad83fb6f9509e362e7c849f57537c5f27fad",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l4t_m8: toff_rp("imchxgrdky4zabCAXZJBMnFsYUt", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,767.585 / 489.
circuit!(
    pf_l4t_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUt", 8, 8, 3),
    [
        "4438238",
        "70854ced688813b8da85abeb3541efa01de279220fed88caf4b6e69f431c3f55",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l4tc_m8: toff_rp("imchxgrdy4zabCAXZJBMnFsYUt", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,708.512 / 497.
circuit!(
    pf_l4tc_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdy4zabCAXZJBMnFsYUt", 8, 8, 3),
    [
        "4430653",
        "aace448cc14821147af18c3c71ad52df92b734cafc5a23f735c500bafc990b3b",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l4tp_m9: toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 18,059.385 / 484.
circuit!(
    pf_l4tp_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 9, 8, 3),
    [
        "4525710",
        "2b849df2c3837fd103208343901604eaba0c39d6aa0911af2f3e2902b58ac008",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l4tp_m8: toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,969.443 / 481.
circuit!(
    pf_l4tp_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 8, 8, 3),
    [
        "4483624",
        "2a9d5c0c2f39e97248f982d47af94b4eb310c056ec8c58e017e029f1f7b7bf6b",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l5p_m9: toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 20,327.454 / 433.
circuit!(
    pf_l5p_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+", 9, 3, 2),
    [
        "4985943",
        "45b98a185d414103d508e74f531b1102e9ca84fdeeac71304cb62e5547b5fb8b",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l5p_m8: toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 20,197.442 / 430.
circuit!(
    pf_l5p_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+", 8, 3, 2),
    [
        "4930148",
        "3356a96c6c1ab90b419e076b2c1b707c18c38cc045f7fa532b2313ec3a917d71",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Lever `C` on the unsplit one-hot (with `E`).

// pf_r1c_m8: toff_rp("imchxlgrdkyECHKVIXt+", 8, 3, 3), reiher-sa-est-v1, keep 8 + 8; full-K 8,722.413 / 528.
circuit!(
    pf_r1c_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyECHKVIXt+", 8, 3, 3),
    [
        "1402339",
        "c83041d7324ac0d5423d5a0e627636171659f0ee963e674596ce7ad88aa2b895",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r1c_m9: toff_rp("imchxlgrdkyECHKVIXt+", 9, 3, 3), reiher-sa-est-v1, keep 9 + 9; full-K 8,791.359 / 532.
circuit!(
    pf_r1c_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxlgrdkyECHKVIXt+", 9, 3, 3),
    [
        "1427180",
        "250492f0083a8460544a1982f80ac8fc2b56c23b1661dc1206d410e91bfa5f3a",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r1cn_m8: toff_rp("imchxgrdkyECHKVIXt+", 8, 3, 3), reiher-sa-est-v1, keep 8 + 8; full-K 8,729.535 / 521.
circuit!(
    pf_r1cn_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyECHKVIXt+", 8, 3, 3),
    [
        "1402519",
        "fcd66297448227da39b6093baa5245dc18b86912732da4cdb10f7904e20b4e4b",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_r1cn_m9: toff_rp("imchxgrdkyECHKVIXt+", 9, 3, 3), reiher-sa-est-v1, keep 9 + 9; full-K 8,799.538 / 524.
circuit!(
    pf_r1cn_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyECHKVIXt+", 9, 3, 3),
    [
        "1427388",
        "e25d9c8d36e0aa8c94224a41dffad301593f8a35fa57e63122f9a352723fb1b3",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// pf_l1c_m9: toff_rp("imchxgrEC", 9, 5, 2), li-sa-est-v1, keep 9 + 9; full-K 13,189.020 / 1,210.
circuit!(
    pf_l1c_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrEC", 9, 5, 2),
    [
        "5669110",
        "600c53f07106049bd8f3ab982b2b0ec8d2a72e66616fce61bd08e20e13a8b20f",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l1c_m8: toff_rp("imchxgrEC", 8, 5, 2), li-sa-est-v1, keep 8 + 8; full-K 13,116.495 / 1,206.
circuit!(
    pf_l1c_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrEC", 8, 5, 2),
    [
        "5555515",
        "a029404fe2f8d6e3485bc06e844223521d8fd37d2d812b06238c8909a4741d1a",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// ---- Compositions of q<b>_1, +, t and C with the earlier letters along the per-budget
// front. Each ops SHA-256 equals its full-K run's (sliced engine, 524,288 lanes, eval OK, after
// the trusted K = 64 check).
// fa_l3q618i9_m9: toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1", 9, 9, 3), li-sa-est-v1, keep 9 + 9; full-K 12,879.514 / 1166.
circuit!(
    fa_l3q618i9_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1", 9, 9, 3),
    [
        "3146828",
        "9f83af58701bf10125355ea5f32bac8f6e583b0b1c20e715db822603bb18e941",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l3q618i9_m8: toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1", 8, 9, 3), li-sa-est-v1, keep 8 + 8; full-K 12,827.411 / 1163.
circuit!(
    fa_l3q618i9_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1", 8, 9, 3),
    [
        "3104602",
        "9fb8d2bb535664dc2abcc20f4f5b55042a0ad99960842b3023cfeaa8be88d90b",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l4tq23_m9: toff_rp("imchxgrdky4zabCAXZJBMnFsYUtq23_1", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,777.409 / 499.
circuit!(
    fa_l4tq23_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUtq23_1", 9, 8, 3),
    [
        "5927350",
        "2d33e9da97622bff52c9408e902944e1e45fe89718830801d1284423bc195eb1",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l4tq26_m8: toff_rp("imchxgrdky4zabCAXZJBMnFsYUtq26_1", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,663.555 / 499.
circuit!(
    fa_l4tq26_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUtq26_1", 8, 8, 3),
    [
        "5865892",
        "ad0926ddfa88289d62b3dc09afad6d6f31762001e4640c63f74dd2cf5eb9ea19",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l4tpa7_m8: toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 8, 7, 3), li-sa-est-v1, keep 8 + 8; full-K 18,907.471 / 480.
circuit!(
    fa_l4tpa7_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 8, 7, 3),
    [
        "4467884",
        "2f375ef36d827ee5a3728f31a6223300cc6c77c43daa6d516510cbec773a7930",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l5pq27_m9: toff_rp("imchxgrdky5zabCHVIXJtRNOBWs+q27_1", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 20,115.470 / 450.
circuit!(
    fa_l5pq27_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWs+q27_1", 9, 3, 2),
    [
        "6371289",
        "d631492aeb4432c1533857bdd60b817b83e5c000baf00b75e0e28602581ea4f0",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l6tpq28_m9: toff_rp("imchxgrdky6zabCHVIXZJsot+q28_1", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 22,165.359 / 430.
circuit!(
    fa_l6tpq28_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsot+q28_1", 9, 3, 2),
    [
        "6431675",
        "e7eeacc4bafe92eae33db1a33de99d45b9ca3dd29902beaf50b394522be130cc",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_l6tpq14_m8: toff_rp("imchxgrdky6zabCHVIXZJsot+q14_1", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 22,245.590 / 413.
circuit!(
    fa_l6tpq14_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsot+q14_1", 8, 3, 2),
    [
        "6416462",
        "7b5abba501c226fdab2dac6af98fa2af211f5ee4b05432812a9727f812f020a2",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// fa_r1tr_m9: toff_rp("imchxgrdkyECHKVIXtRNOBW+", 9, 3, 3), reiher-sa-est-v1, keep 9 + 9; full-K 8,827.527 / 520.
circuit!(
    fa_r1tr_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyECHKVIXtRNOBW+", 9, 3, 3),
    [
        "1427828",
        "fdbdf8dc7b62253999d95893b318e36affe781c08cbff2df2b4746e201b1dc00",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r1tr_m8: toff_rp("imchxgrdkyECHKVIXtRNOBW+", 8, 3, 3), reiher-sa-est-v1, keep 8 + 8; full-K 8,755.543 / 517.
circuit!(
    fa_r1tr_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdkyECHKVIXtRNOBW+", 8, 3, 3),
    [
        "1402933",
        "82244d6c2e20b8461b3a2e478617b21613149c364ad623c88c19eca18492f40f",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r3pq150_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q150_1", 9, 3, 3), reiher-sa-est-v1, keep 9 + 9; full-K 9,694.564 / 453.
circuit!(
    fa_r3pq150_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q150_1", 9, 3, 3),
    [
        "1245812",
        "bea46c39232c28f1a33e7362d01444fb6c57dcdc5b91d98b21b4392e954f745f",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r3pq153_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q153_1", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 9,600.690 / 453.
circuit!(
    fa_r3pq153_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q153_1", 8, 2, 2),
    [
        "1208492",
        "b4b93bef810117f8e1fd3d62d0bd27597c9b6ddc6d0acadd283d8c4666abe91c",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r4s_m8: toff_rp("imchxgrdky4zZabCHKVITXSRNOBWs", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 14,305.525 / 285.
circuit!(
    fa_r4s_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabCHKVITXSRNOBWs", 8, 2, 2),
    [
        "1532537",
        "019f58088d4a082f8079ec009c1178fc27868cd448aa3cae341039bed7eb6157",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r7s_m8: toff_rp("imchxgrdky7zabfHKVITXSRNOBWUs", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 19,294.377 / 253.
circuit!(
    fa_r7s_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabfHKVITXSRNOBWUs", 8, 2, 1),
    [
        "1555616",
        "a0ace9f19aa86b2ab655ccd4d41baa32e09c1332e9165ab90802d798a642eb39",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r7s_m9: toff_rp("imchxgrdky7zabfHKVITXSRNOBWUs", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 19,417.332 / 256.
circuit!(
    fa_r7s_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabfHKVITXSRNOBWUs", 9, 2, 1),
    [
        "1578619",
        "d0221c401b768f2b43e9fd78a2d8e77737448168a22d7e6922f2a7cc3eefc8f0",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// The Li point pf_l4tp extended. Levers `.p` (padding runs: the paired read's pruned slot one-hot), `.g` (the
// trailing one-body sub-row class grafted into spare one-hot cells), `.h4` (U's hold capped at
// the Majorana), with `U` added and `e` dropped (the read stage no longer binds).
// kl_l4pgh_m9: toff_rp("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,839.592 / 480.
circuit!(
    kl_l4pgh_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 9, 8, 3),
    [
        "4484556",
        "9a6569de697e85e01640d3abf4202534b35faa3af2fb63cc158861f3f7b69247",
        "16fb8e8a647a7d4a72a3b3ce02b285f109969924be4f342243df1543797b3a91",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l4pgh_m8: toff_rp("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,749.672 / 477.
circuit!(
    kl_l4pgh_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 8, 8, 3),
    [
        "4442322",
        "f3ff2b2a6b263a120e7b67ebe42c060625326376a33b5ce239a3d7de0c1c5276",
        "7c23ebefcc20c035ab4c62f1879b8f2304174340d0400de861a4cb9cfb4acd5e",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// ---- Lever `-`, the folded item one-hot read ---------------------------------------------------
// kn_l5f_m9: toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+-", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 19,937.383 / 433.
circuit!(
    kn_l5f_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+-", 9, 3, 2),
    [
        "5023697",
        "10e030735b6aa555112fb92903beb09eed9fcf8ac966969902761bfd9d646c45",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l5f_m8: toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+-", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 19,863.404 / 430.
circuit!(
    kn_l5f_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHVIXJtRNOBWPs+-", 8, 3, 2),
    [
        "4969896",
        "fb48129fd0a19f2d60721543311542d7a9d8138f80fb2c34628640f5401a9c41",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l4f_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 17,963.427 / 483.
circuit!(
    kn_l4f_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-", 9, 3, 2),
    [
        "4807105",
        "1c38687f8f9ab3020c0bde3d7ba2d59e354836988c950f4292e4aace101bd593",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l4f_m8: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 17,889.569 / 480.
circuit!(
    kn_l4f_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-", 8, 3, 2),
    [
        "4753304",
        "b01d9328df60c02df8e3d304f005c7cad7ebbc3a68db86624c3f06bcc7d71886",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l5fh_m9: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 19,965.500 / 427.
circuit!(
    kn_l5fh_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_", 9, 3, 2),
    [
        "5026945",
        "078fcee777429c35cf5507752558311dd3845a5a0684304454de5fd8f31e3031",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l5fh_m8: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 19,891.463 / 424.
circuit!(
    kn_l5fh_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_", 8, 3, 2),
    [
        "4973144",
        "b5db89cfbb29c47e109924ffc9bb95f1d962a3e14adbd84c31531a0585722cd1",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l4fh_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 17,987.408 / 477.
circuit!(
    kn_l4fh_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 2),
    [
        "4807213",
        "95921b2b0217cebd69e75f9b2b2fbdc37224d01d0c9231cb4d7b2a2e8dd9904d",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// kn_l4fh_m8: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 17,913.426 / 474.
circuit!(
    kn_l4fh_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 8, 3, 2),
    [
        "4753412",
        "b740d7fd46bbdf88505d75f1026e9346c2b4c22f6e0d4747cfb9d09ce7295f54",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// pf_l4tp without `R`.
// kl_l4r_m9: toff_rp("imchxgrdky4zabCAXZJtNOBWMenFsY", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 18,041.552 / 484.
circuit!(
    kl_l4r_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtNOBWMenFsY", 9, 8, 3),
    [
        "4525470",
        "927121286ab9ba972e86f4e38785cb6dfd7feff131b68c69bb154b3dbef2a422",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l4r_m8: toff_rp("imchxgrdky4zabCAXZJtNOBWMenFsY", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,953.538 / 481.
circuit!(
    kl_l4r_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtNOBWMenFsY", 8, 8, 3),
    [
        "4483414",
        "346b04ce949f950083538d9ea896936d2ea478b34c7b599d08988f5b539dae7c",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Composed with `-` and `_`: `.g`, `U .h4`.
// kl_l4hfg_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh4", 9, 3, 3), li-sa-est-v1, keep 9 + 9; full-K 17,915.514 / 474.
circuit!(
    kl_l4hfg_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh4", 9, 3, 3),
    [
        "4781341",
        "71d6dde41494b0dcb027b9c8999aefbf03a701b0e26c2b84138fddb2747ab1f2",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l4hfg_m8: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh4", 8, 3, 3), li-sa-est-v1, keep 8 + 8; full-K 17,837.591 / 471.
circuit!(
    kl_l4hfg_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh4", 8, 3, 3),
    [
        "4727217",
        "0da027579c4c992b853c82158df3b654eccbdb40309dfcd9e6b52cbff77b5947",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// G = 5 composed: kn_l5fh + `U .h4`.
// kl_l5fhh_m9: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.h4", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 19,925.493 / 427.
circuit!(
    kl_l5fhh_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.h4", 9, 3, 2),
    [
        "4988305",
        "71f19059acfe2c2283931992ddb380be7b4a0252d617ffafd40a96e6ea5863eb",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l5fhh_m8: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.h4", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 19,851.658 / 424.
circuit!(
    kl_l5fhh_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.h4", 8, 3, 2),
    [
        "4934504",
        "1bb2ac26291e0e559765d84e1b6ba1a695a6ee67ef2cb0bc448374c0ad439c98",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// The paired-read point with `_` and `e` back.
// kl_l4pu_m9: toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsYU_.pgh4", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,995.479 / 474.
circuit!(
    kl_l4pu_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsYU_.pgh4", 9, 8, 3),
    [
        "4486510",
        "b34997ffa9a6d47a9628a3c949604e58aaccafa2773e7548a5193f975cc2569e",
        "16fb8e8a647a7d4a72a3b3ce02b285f109969924be4f342243df1543797b3a91",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l4pu_m8: toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsYU_.pgh4", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,905.362 / 471.
circuit!(
    kl_l4pu_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsYU_.pgh4", 8, 8, 3),
    [
        "4444276",
        "af30ca89a0022e5e9cfc96133e81da0822534b88cda5ffbfe43a1c4544aa8a20",
        "7c23ebefcc20c035ab4c62f1879b8f2304174340d0400de861a4cb9cfb4acd5e",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Transfer down the Li front: G = 3 (rdl-L3s + `.pgh4 U - e`).
// kl_l3pgh_m9: toff_rp("imchxgrdky3zabCAXJSRNOBWMnFsU.pgh4", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 16,039.470 / 555.
circuit!(
    kl_l3pgh_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJSRNOBWMnFsU.pgh4", 9, 8, 3),
    [
        "4404552",
        "f17c015335726dfaaa3b13eb6a8aa3c11d729965b425c94b19e6277f55ae293a",
        "16fb8e8a647a7d4a72a3b3ce02b285f109969924be4f342243df1543797b3a91",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// Transfer: the low-C end (fa_l3q618i9 + `.p`).
// kl_l3q618p_m9: toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1.p", 9, 9, 3), li-sa-est-v1, keep 9 + 9; full-K 12,821.482 / 1,166.
circuit!(
    kl_l3q618p_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1.p", 9, 9, 3),
    [
        "3132504",
        "6bcb6a20a59e1c1d9f8cfc8b1e144066c829e825d0057818106fa855f0bc58a1",
        "16fb8e8a647a7d4a72a3b3ce02b285f109969924be4f342243df1543797b3a91",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// The composed point with lever `.e` (the paired phase pass).
// kl_l4hfge_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4", 9, 3, 3), li-sa-est-v1, keep 9 + 9; full-K 17,855.560 / 474.
circuit!(
    kl_l4hfge_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4", 9, 3, 3),
    [
        "5256599",
        "ef431759463cb8e2461096cdf2d85a6a7a692a92a7c881d699cb2a8fab781701",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l4hfge_m8: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4", 8, 3, 3), li-sa-est-v1, keep 8 + 8; full-K 17,777.618 / 471.
circuit!(
    kl_l4hfge_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4", 8, 3, 3),
    [
        "5184265",
        "27c82e2179cd4595bf9d91c03e3c2ba496b998f8e6a63f1d9a96db7243f049b2",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// G = 5 composed with `.e`.
// kl_l5fhe_m9: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 19,865.437 / 427.
circuit!(
    kl_l5fhe_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4", 9, 3, 2),
    [
        "5463563",
        "58e80903694c98879a65d9e9d80d8f8a5e5f9de9eadc21b06d4a9ca225810188",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kl_l5fhe_m8: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 19,791.416 / 424.
circuit!(
    kl_l5fhe_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4", 8, 3, 2),
    [
        "5391552",
        "831604c2ba5dbeaab532c13a86c64b24fce9493f832f70caff613551e2797648",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// `.e` down the front: rdl-L7q22 + `.e`.
// kl_l7e_m9: toff_rp("imchxgrdky7zabCHVIXJsoq22_1.e", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 24,761.357 / 400.
circuit!(
    kl_l7e_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHVIXJsoq22_1.e", 9, 3, 2),
    [
        "7179003",
        "376fff221a75ce7f3cd10ab7671d14f290476abea6084a4937a5a85b3b235a7c",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// `.e` down the front: fa_l6tpq28 + `.e`.
// kl_l6e_m9: toff_rp("imchxgrdky6zabCHVIXZJsot+q28_1.e", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 22,105.321 / 430.
circuit!(
    kl_l6e_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJsot+q28_1.e", 9, 3, 2),
    [
        "6906933",
        "6647e90cfe7935f0d329a2f458482c475166932fc33ddbff4dc8069c91aa12d0",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4fh3_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 3), li-sa-est-v1, keep 9 + 9; full-K 17,977.585 / 477.
circuit!(
    kn_l4fh3_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 3),
    [
        "4809617",
        "e1f131e4256c76d8b4a819ee568e2acf0907a75d06aeceb89ca9f30effcf046f",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4fh3_m8: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 8, 3, 3), li-sa-est-v1, keep 8 + 8; full-K 17,899.436 / 474.
circuit!(
    kn_l4fh3_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 8, 3, 3),
    [
        "4755493",
        "7d94d84c0243b3d75b5417c7b5f4b98089537bfd79c4e8500537ebbddf5d6e9e",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l5fhq30_m9: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q30_1", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 19,717.878 / 447.
circuit!(
    kn_l5fhq30_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q30_1", 9, 3, 2),
    [
        "6407893",
        "c2834b4fcc6cd760dda6c12d5dbf98d4e216303195746d96b5a4be7342dd64e9",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l5fhq36_m8: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q36_1", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 19,571.656 / 450.
circuit!(
    kn_l5fhq36_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q36_1", 8, 3, 2),
    [
        "6330788",
        "454c6d521465af04c882df1996e5e7103749cd5b1e897200d07b94bd50eea114",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l5fhq50_m9: toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q50_1", 9, 3, 3), li-sa-est-v1, keep 9 + 9; full-K 19,467.545 / 467.
circuit!(
    kn_l5fhq50_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPs+-_q50_1", 9, 3, 3),
    [
        "6313103",
        "acd7fbe858b5531153e99543209d9a40dcd08a8fa4e1bbbf3bedf350eadb7513",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4fhq24_m9: toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_q24_1", 9, 3, 3), li-sa-est-v1, keep 9 + 9; full-K 17,849.253 / 491.
circuit!(
    kn_l4fhq24_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_q24_1", 9, 3, 3),
    [
        "6205797",
        "3e3bea6ff03c35412e96678d001a24943b0188187f4879597baf743c2d0b93f2",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4phq28_m9: toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q28_1", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,757.416 / 498.
circuit!(
    kn_l4phq28_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q28_1", 9, 8, 3),
    [
        "5891580",
        "c10cc7527b15fc29e7b61c0fb4914ddeac9161eca9c8bb6862cb7fff30baae11",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4phq32_m8: toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q32_1", 8, 8, 3), li-sa-est-v1, keep 8 + 8; full-K 17,633.494 / 499.
circuit!(
    kn_l4phq32_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q32_1", 8, 8, 3),
    [
        "5853732",
        "def5ceebc39f0af19a224a0a3c84d0b2b638abbc5503b148e6c11a93bc63d03b",
        "d301d96ca086cee8487049f543c4cafbfa7e533161cc770e6b81a49762e428a6",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kn_l4phq50_m9: toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q50_1", 9, 8, 3), li-sa-est-v1, keep 9 + 9; full-K 17,559.523 / 520.
circuit!(
    kn_l4phq50_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q50_1", 9, 8, 3),
    [
        "5809986",
        "8c8c407732fc48a7d45f94bf68166ec8de93e72a448a2624e0ef3f3377253cbb",
        "e6205e1bde758322f4a66db1793a82841ccc0ad96c5823edff405d3cc2465db3",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);

// kr_r3m_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,598.640 / 316.
circuit!(
    kr_r3m_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13", 9, 2, 2),
    [
        "1772069",
        "c96fe8ef90d8bd23485d6f4908ae8d295af20259b7428ed4fe2f5f9615e9f822",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3m_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,532.603 / 313.
circuit!(
    kr_r3m_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13", 8, 2, 2),
    [
        "1747400",
        "77c12ee45984ea212c6877230f43369aab6f615d8e7e70acaf730af56bf51a13",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q17_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q17_1.14", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,580.580 / 317.
circuit!(
    kr_r3q17_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q17_1.14", 9, 2, 2),
    [
        "1767599",
        "2a951ebf73a7b0bf72aad47491353e1bce6dfe6eb039dcaa076103a23d44b075",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q18_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,562.628 / 318.
circuit!(
    kr_r3q18_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15", 9, 2, 2),
    [
        "1763645",
        "e82d89b41e28df4db9526da4c876e85eaf40d20f1cf332944ee74cdf7f91b981",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q23_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,492.274 / 323.
circuit!(
    kr_r3q23_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20", 9, 2, 2),
    [
        "1746513",
        "1a9731716ac3322e4b7e28b285638a62cef24e192479512b349b568a29571eaf",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q33_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,352.461 / 333.
circuit!(
    kr_r3q33_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30", 9, 2, 2),
    [
        "1705109",
        "230ae26b0edebdbfa455b8af7ee54d1b2352c4824b91f3db500445a781c38e18",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q18_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,496.358 / 315.
circuit!(
    kr_r3q18_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15", 8, 2, 2),
    [
        "1738976",
        "7789c61c5b4127de4867a4d4060da3ea1e3a95ec2119db36fd334ff1be1816c4",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q23_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,426.435 / 320.
circuit!(
    kr_r3q23_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20", 8, 2, 2),
    [
        "1721844",
        "abe84321faf3d5fd6d5a83f37e54ab8f5947c2552ffccd744b0481a3656e20e6",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// kr_r3q33_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,286.489 / 330.
circuit!(
    kr_r3q33_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30", 8, 2, 2),
    [
        "1680440",
        "1bbb082b27e4a9c3c22cfcab26fb6c6b5f4c99465544b3495bd1848a775cb346",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// q<b>_1.<c> composed with `.e` (the paired phase pass).
// kc_r3e_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,568.566 / 316.
circuit!(
    kc_r3e_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.e", 9, 2, 2),
    [
        "1961531",
        "717f6608f59c1d85b5d8b2c195cfadd6a14a6116fc3bab2ef350fe1761a6aaf5",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3re_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,568.588 / 316.
circuit!(
    kc_r3re_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e", 9, 2, 2),
    [
        "1957301",
        "26d2cea001177278fbba67522d81c95642c7856648f40862c80c075a6c4fc841",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3e_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,502.540 / 313.
circuit!(
    kc_r3e_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.e", 8, 2, 2),
    [
        "1927460",
        "d28046df9177ce526fbbd2484c9052f55a58afd0c561af2083b6dc282e121161",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3re_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,500.594 / 313.
circuit!(
    kc_r3re_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e", 8, 2, 2),
    [
        "1923200",
        "490a9c92e7a1c1ad1647f54b4a218ab797991a0a659a2100bc96328c20310c93",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q17e_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q17_1.14.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,550.506 / 317.
circuit!(
    kc_r3q17e_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q17_1.14.e", 9, 2, 2),
    [
        "1957061",
        "851cdd3e007854b81b5fac054081e7b609a1895db2e9ecb07c280813e46e5c20",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q18e_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,532.603 / 318.
circuit!(
    kc_r3q18e_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15.e", 9, 2, 2),
    [
        "1953107",
        "abba3800c0099ac673c8300b9ac86ba4e07eb3bf08132129bf684f69ba745622",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q23e_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,462.537 / 323.
circuit!(
    kc_r3q23e_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20.e", 9, 2, 2),
    [
        "1935975",
        "a173d0411c4c8e72eae9429fbb657ede900a9fb92e3c52613455699da43e8330",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q33e_m9: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30.e", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 11,322.326 / 333.
circuit!(
    kc_r3q33e_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30.e", 9, 2, 2),
    [
        "1894571",
        "8ff64f41c38faf3dfc15340caab53d80c84283c46af8abac02c1960603f850e1",
        "c4a16bd7d3f964905450e383f2432097e5c9f22a24026694880db53a38f71034",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3rq18e_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq18_1.15.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,482.523 / 314.
circuit!(
    kc_r3rq18e_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+Rq18_1.15.e", 8, 2, 2),
    [
        "1919246",
        "4275b3afcfdaf406e7702d09da285750fa50818e273d89203b0cb562bbeae86d",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q18e_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,466.471 / 315.
circuit!(
    kc_r3q18e_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q18_1.15.e", 8, 2, 2),
    [
        "1919036",
        "b7861a4b0f899ce3b4afe97d8f96982f62edb8f3e7c592111d4d76b132b44e75",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q23e_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,396.471 / 320.
circuit!(
    kc_r3q23e_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q23_1.20.e", 8, 2, 2),
    [
        "1901904",
        "bc9be9411cbd9403a88895ac99fed69d905b9909c75bfd0a4ab727a718ffca3d",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// kc_r3q33e_m8: toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30.e", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 11,256.376 / 330.
circuit!(
    kc_r3q33e_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVIDXtNBWs+q33_1.30.e", 8, 2, 2),
    [
        "1860500",
        "a7bcb005023b54e809ccbddcfc4f6b37d3ee0a42c2b20801e9aaefdf2aafe023",
        "6729a2f41799a88814c0b4690d8799aa1cfef43af7dd18ba83c081fab4d0e286",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ---- Lever `~` ----
// gkr_r5_m9: toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,820.173 / 256.
circuit!(
    gkr_r5_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~", 9, 2, 1),
    [
        "1719091",
        "05ebe873d7991bc444318d6fe2b0acba62481c39ec441f16d87ca691020c9c71",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r5_m8: toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,642.580 / 253.
circuit!(
    gkr_r5_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~", 8, 2, 1),
    [
        "1674272",
        "cbba4fbdf160e05a4776e6cbb76d7755ac4b1b8679d3786c2708aaf1592a85b1",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r4zf_m9: toff_rp("imchxgrdky4zZabfHKVITXSNOBWs_~", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 15,429.471 / 270.
circuit!(
    gkr_r4zf_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHKVITXSNOBWs_~", 9, 2, 1),
    [
        "1694335",
        "98d0b783a327469b37fa6c911e64e8f4375b63bf2c053f3f1e899eb6daa275ec",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r4zf_m8: toff_rp("imchxgrdky4zZabfHKVITXSNOBWs_~", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 15,252.726 / 268.
circuit!(
    gkr_r4zf_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHKVITXSNOBWs_~", 8, 2, 1),
    [
        "1649516",
        "04436c0b2e68b0753298fb3c81fbbd857d07c22e96a27fa1b4462ad4037be9a1",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r3c_m9: toff_rp("imchxgrdky3zabCHKVITXSNOBWs_~", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 13,402.508 / 300.
circuit!(
    gkr_r3c_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVITXSNOBWs_~", 9, 2, 2),
    [
        "1621928",
        "fd94f6b94a3e722de079aab94aa0c21e2bd9daa5bb8bf847cef6a53297bbd80a",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r3c_m8: toff_rp("imchxgrdky3zabCHKVITXSNOBWs_~", 8, 2, 2), reiher-sa-est-v1, keep 8 + 8; full-K 13,223.304 / 298.
circuit!(
    gkr_r3c_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky3zabCHKVITXSNOBWs_~", 8, 2, 2),
    [
        "1576825",
        "54e4c70e638bd1526fe3ace62e92481d39e662526cd1717912f32dfa52975848",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_r5u_m8: toff_rp("imchxgrdky5zabfHKVITXSNOBWUs_~", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,584.080 / 256.
circuit!(
    gkr_r5u_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWUs_~", 8, 2, 1),
    [
        "1653980",
        "594ad6ca1488551e9826c7155b2679fd778347ae0503c2dfaaa9fbeff02ddfa8",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// gkr_l6_m9: toff_rp("imchxgrdky6zabCHVIXZJNOBWsot+_~", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 23,090.800 / 396.
circuit!(
    gkr_l6_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJNOBWsot+_~", 9, 3, 2),
    [
        "5529727",
        "3207b080f90e910667579457507327da88813c31034dbe2c7d7d29920092b548",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// gkr_l6_m8: toff_rp("imchxgrdky6zabCHVIXZJNOBWsot+_~", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 22,903.997 / 394.
circuit!(
    gkr_l6_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHVIXZJNOBWsot+_~", 8, 3, 2),
    [
        "5419824",
        "0cea8ab5ea50969f29287d3c2f32afa5d8da7f5e860e684c2e08a4bb97fd0709",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// gkr_l7_m9: toff_rp("imchxgrdky7zabCHVIXJNOBWsot+_~", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 25,509.317 / 384.
circuit!(
    gkr_l7_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHVIXJNOBWsot+_~", 9, 3, 2),
    [
        "5564055",
        "c7934830e6211c84532541b65f99088c5dd78e9ec41a7cd423b934beea70aff8",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// gkr_l7_m8: toff_rp("imchxgrdky7zabCHVIXJNOBWsot+_~", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 25,322.665 / 381.
circuit!(
    gkr_l7_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHVIXJNOBWsot+_~", 8, 3, 2),
    [
        "5454152",
        "bd187e1cd687bad358a2228dceb2eb30a41e65899691b68e50a43332e4e29dec",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// ctl_r6_m8: toff_rp("imchxgrdky6zZabfHKVITXSRNOBWUs_", 8, 1, 1), reiher-sa-est-v1, keep 8 + 8; full-K 17,870.363 / 253.
circuit!(
    ctl_r6_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabfHKVITXSRNOBWUs_", 8, 1, 1),
    [
        "1549428",
        "a8dd9513f429ec6a95d99850c35903a9187ad6f9479bafa5e286961d0dc2abbd",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r6_m9: toff_rp("imchxgrdky6zZabfHKVITXSRNOBWUs_", 9, 3, 1), reiher-sa-est-v1, keep 9 + 9; full-K 17,993.431 / 256.
circuit!(
    ctl_r6_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabfHKVITXSRNOBWUs_", 9, 3, 1),
    [
        "1572431",
        "37da939941a2da0fbaae377e5f5b979015228b39fba9769e8ad43c92e09a6cd2",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5r_m8: toff_rp("imchxgrdky5zabfHKVITXSNOBWsR_", 8, 1, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,092.549 / 259.
circuit!(
    ctl_r5r_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWsR_", 8, 1, 1),
    [
        "1537004",
        "0c6309bf72b7dfb1eef1ade0a6dfb47f787988ceb00b068139442fe0da455935",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5r_m9: toff_rp("imchxgrdky5zabfHKVITXSNOBWsR_", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,215.469 / 262.
circuit!(
    ctl_r5r_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWsR_", 9, 2, 1),
    [
        "1560007",
        "5d06ada6cff8069aa713a3ac71c0ecafd6aff105f2874420f49ae09b70a10669",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5s_m8: toff_rp("imchxgrdky5zabfHKVITXSNOBWs_", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,076.658 / 260.
circuit!(
    ctl_r5s_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWs_", 8, 2, 1),
    [
        "1536794",
        "bf018cc66d547e06d089f13fedfb071e9dcd62f8f83175882ca9ce8b855062ea",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5s_m9: toff_rp("imchxgrdky5zabfHKVITXSNOBWs_", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,197.581 / 263.
circuit!(
    ctl_r5s_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWs_", 9, 2, 1),
    [
        "1559767",
        "2cbabf5801dcbd1bd67cdf182dedd8eed9b95034e9f907597217140f388fb60e",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5n_m8: toff_rp("imchxgrdky5zabfHVITXSNOBWs_", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,040.462 / 261.
circuit!(
    ctl_r5n_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHVITXSNOBWs_", 8, 2, 1),
    [
        "1535312",
        "13811467b0986355471b85075c78383f72ffbe1de39af91a847fe3ed11052d21",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r5n_m9: toff_rp("imchxgrdky5zabfHVITXSNOBWs_", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,161.734 / 264.
circuit!(
    ctl_r5n_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHVITXSNOBWs_", 9, 2, 1),
    [
        "1558285",
        "431863ed96b5c9156ef49e40d392d03a1104bd67ec780b621b4ff198e7dcbdea",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r4_m8: toff_rp("imchxgrdky4zZabfHKVITXSRNOBWs_", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 14,702.619 / 275.
circuit!(
    ctl_r4_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHKVITXSRNOBWs_", 8, 2, 1),
    [
        "1512248",
        "c152be62a572e9a38bf8fc8e2d047c3da658bc2cb03b9581c87355b46f78043e",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// ctl_r4_m9: toff_rp("imchxgrdky4zZabfHKVITXSRNOBWs_", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 14,825.629 / 278.
circuit!(
    ctl_r4_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHKVITXSRNOBWs_", 9, 2, 1),
    [
        "1535251",
        "7171d788c8003caed1bf1f41378ddf6450cada62568c8757ab4230a42a904a9c",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r255_m8: toff_rp("imchxgrdky6zZabfHKVITXSRNOBWs", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 17,926.407 / 255 (the baseline without `~`).
circuit!(
    fa_r255_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabfHKVITXSRNOBWs", 8, 2, 1),
    [
        "1568440",
        "7e951f7a0f7c7d64f363b130aa9ea2bc147ee96ab827afc9c5fb50d9fe0a90ef",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r260_m8: toff_rp("imchxgrdky6zZabCHKVITXSRNOBWs", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 17,612.444 / 260 (the baseline without `~`).
circuit!(
    fa_r260_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabCHKVITXSRNOBWs", 8, 2, 1),
    [
        "1594198",
        "0970971c6d3b002c38467214685e979d4041aadecd1f5ae190bd5593b05bf2b1",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r264_m8: toff_rp("imchxgrdky5zabCHKVITXSRNOBWs", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,086.692 / 264 (the baseline without `~`).
circuit!(
    fa_r264_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVITXSRNOBWs", 8, 2, 1),
    [
        "1531570",
        "3ba81d0a73ca62e7b79d64d847b4f1816bced9adadb952a5745c5c94086b1f66",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r258_m9: toff_rp("imchxgrdky6zZabfHKVITXSRNOBWs", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 18,049.615 / 258 (the baseline without `~`).
circuit!(
    fa_r258_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabfHKVITXSRNOBWs", 9, 2, 1),
    [
        "1591443",
        "5d24243e3fd0e5dc1d612f5cf26547d246467f433542f597e317f716818002bd",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r263_m9: toff_rp("imchxgrdky6zZabCHKVITXSRNOBWs", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 17,735.369 / 263 (the baseline without `~`).
circuit!(
    fa_r263_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zZabCHKVITXSRNOBWs", 9, 2, 1),
    [
        "1617201",
        "f77b387ecc5b06991127f011b6ab63bceadc19cb1445a5c34fad09dcc2741729",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r267_m9: toff_rp("imchxgrdky5zabCHKVITXSRNOBWs", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,209.516 / 267 (the baseline without `~`).
circuit!(
    fa_r267_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVITXSRNOBWs", 9, 2, 1),
    [
        "1554573",
        "bb999a23932db3cec69196922c056005d0575c54d8fea27465a96cc124150e87",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// fa_r288_m9: toff_rp("imchxgrdky4zZabCHKVITXSRNOBWs", 9, 2, 2), reiher-sa-est-v1, keep 9 + 9; full-K 14,429.658 / 288 (the baseline without `~`).
circuit!(
    fa_r288_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabCHKVITXSRNOBWs", 9, 2, 2),
    [
        "1555824",
        "6e2303672e63f2c9861eeb077529b24bd18d3a6ae58b04f3f58434f668b6af79",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);

// ---- Lever `~` composed with `q<b>_1.<c>` and `.e` / `.eh4`.
// Every circuit full-K 'eval OK' (524,288 lanes, sliced engine).
// rc_r5q_m9: toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~q16_1.13", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 16,704.888 / 256.
circuit!(
    rc_r5q_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWs_~q16_1.13", 9, 2, 1),
    [
        "2095419",
        "655b43a44ba62729fe1cded5c8c593f0ff78e61aa3149300f75a547329d405ff",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rc_r5q_m8: toff_rp("imchxgrdky5zabfHKVITXSNOBWUs_~q15_1.12", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 16,553.890 / 253.
circuit!(
    rc_r5q_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabfHKVITXSNOBWUs_~q15_1.12", 8, 2, 1),
    [
        "2053698",
        "75ced199821226de75ffefb0076539f65cf2f5c7d81fbfd4bf1137c5cc894d4a",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rc_r4q_m9: toff_rp("imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12", 9, 2, 1), reiher-sa-est-v1, keep 9 + 9; full-K 15,317.819 / 270.
circuit!(
    rc_r4q_m9,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12", 9, 2, 1),
    [
        "2059795",
        "292bab486bb825512342668c0f47a5d7b6aa6cbfd0a9cb523b5c06b6ccd33f5f",
        "96cbc8275a617d0018869647b216a619b807bf264a62d6b05be9dae14a75bd43",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rc_r4q_m8: toff_rp("imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12", 8, 2, 1), reiher-sa-est-v1, keep 8 + 8; full-K 15,140.515 / 268.
circuit!(
    rc_r4q_m8,
    "reiher-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12", 8, 2, 1),
    [
        "2014976",
        "bcd0c7a83b2917800d660b59a164fa9b43f161b7e884d82627e16e4367211de8",
        "4442f4cef5093e7b891c589e3608c0291684fc11127848956f83d237bde0fefe",
        "204e996c829b0d287c97afce76b09019e34569b13aa5ceca1f3a5cd4f9d1ea3a",
    ]
);
// rc_l7_m9: toff_rp("imchxgrdky7zabCHKVIXJOBWsotU+_~.e", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 25,367.595 / 377.
circuit!(
    rc_l7_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHKVIXJOBWsotU+_~.e", 9, 3, 2),
    [
        "6153163",
        "25edc76d73cd623cd15a5262e5330c0a996060bde2d5c1ad1a27a1d5e9b7a827",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rc_l7_m8: toff_rp("imchxgrdky7zabCHKVIXJOBWsotU+_~.e", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 25,180.568 / 374.
circuit!(
    rc_l7_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky7zabCHKVIXJOBWsotU+_~.e", 8, 3, 2),
    [
        "6006830",
        "7c272fcb5df70c9a5899d1c892ec685968ce630ecfdb87707c1b0d7db099327f",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rc_l6_m9: toff_rp("imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 22,986.093 / 395.
circuit!(
    rc_l6_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4", 9, 3, 2),
    [
        "6130765",
        "f36160844b6497451be121ec871f0ba04b087cc24a12483d7069362aef232996",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rc_l6_m8: toff_rp("imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 22,798.500 / 393.
circuit!(
    rc_l6_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4", 8, 3, 2),
    [
        "5984432",
        "6ff2fab3454c74b41aa8b79401943cad1e86179e9c209c0db353be088ffd5acb",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rc_l5_m9: toff_rp("imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4", 9, 3, 2), li-sa-est-v1, keep 9 + 9; full-K 20,883.234 / 419.
circuit!(
    rc_l5_m9,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4", 9, 3, 2),
    [
        "6027825",
        "75d99c457ada44bfe14d4b0d739541cc8a29f8f52cefe0484a2f244a8ed0634a",
        "90b9dbac7c5a4fc114a39162f773cb9d53481e886e9f0de54aec6164cfe98d05",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
// rc_l5_m8: toff_rp("imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4", 8, 3, 2), li-sa-est-v1, keep 8 + 8; full-K 20,696.657 / 417.
circuit!(
    rc_l5_m8,
    "li-sa-est-v1",
    sa_low::family_toff(),
    toff_rp("imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4", 8, 3, 2),
    [
        "5881492",
        "c2e2c1c3fb36a0b86ec0cc329b3673f60ec235c0d2f45e65df1beb9aca4952ca",
        "d07ab2fe582d89f6565f7ff084626bf8821c38a762f52ba28db402ae7d6f0743",
        "365285405bb23d4409a73a4e6ec4cfc577b5a193c5d0269b00c7bd40a5eeecf5",
    ]
);
