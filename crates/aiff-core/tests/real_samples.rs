//! Tests against real Serato recordings.
//!
//! `committed_head_slice_*` runs everywhere. The `#[ignore]` tests need the full sample
//! files in the repo root (gitignored); run them with `cargo test -p aiff-core -- --ignored`.

use aiff_core::{diagnose_path, repair_copy, Defect, Health};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn committed_head_slice_of_the_real_broken_file_is_repairable() {
    let src = repo_root().join("tests/fixtures/serato-broken-head.aif");
    let d = diagnose_path(&src).unwrap();
    assert_eq!(d.format.sample_rate, 48000.0);
    assert_eq!((d.format.channels, d.format.bits), (2, 16));
    assert_eq!(d.frames, 262_144);
    let Health::Broken(defects) = &d.health else {
        panic!("should be broken")
    };
    assert!(defects
        .iter()
        .any(|x| matches!(x, Defect::FormSize { found: 0, .. })));

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("fixed.aif");
    repair_copy(&src, &dst, &mut |_, _| true).unwrap();
    let fixed = diagnose_path(&dst).unwrap();
    assert!(fixed.is_healthy());
    assert_eq!(fixed.frames, 262_144);
}

#[test]
#[ignore = "needs the full sample files in the repo root"]
fn good_samples_are_healthy_with_id3_tags() {
    for (name, frames) in [
        ("Serato Recording 1.aif", 476_280_064u64),
        ("Serato Recording 2.aif", 211_684_864),
    ] {
        let d = diagnose_path(&repo_root().join(name)).unwrap();
        assert!(d.is_healthy(), "{name}: {:?}", d.health);
        assert_eq!(d.frames, frames, "{name}");
        assert_eq!(d.format.sample_rate, 44100.0);
        assert!(
            d.tags.as_ref().and_then(|t| t.recorded.as_ref()).is_some(),
            "{name} has a Recorded tag"
        );
    }
}

#[test]
#[ignore = "needs the full sample files in the repo root"]
fn the_real_broken_sample_is_diagnosed_and_repaired() {
    let src = repo_root().join("Serato Recording 5.aif");
    let d = diagnose_path(&src).unwrap();
    assert_eq!(d.frames, 55_121_152);
    assert!(!d.is_healthy());

    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("fixed.aif");
    repair_copy(&src, &dst, &mut |_, _| true).unwrap();
    let fixed = diagnose_path(&dst).unwrap();
    assert!(fixed.is_healthy());
    assert_eq!(fixed.frames, 55_121_152);
}
