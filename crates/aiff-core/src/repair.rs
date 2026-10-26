//! Repair a diagnosed-broken file by patching its header fields.

use crate::{diagnose_path, Diagnosis, Error, Progress, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const COPY_BUF: usize = 1 << 20;

fn diagnose_broken(path: &Path) -> Result<Diagnosis> {
    let d = diagnose_path(path)?;
    if d.is_healthy() {
        return Err(Error::NotBroken);
    }
    let f = &d.fixed;
    if [f.form_size, f.ssnd_size, f.frames]
        .iter()
        .any(|&v| v > u64::from(u32::MAX))
    {
        return Err(Error::TooLarge);
    }
    Ok(d)
}

/// Write the three size fields the way a clean close would have.
fn patch_header<W: Write + Seek>(w: &mut W, d: &Diagnosis) -> Result<()> {
    for (pos, value) in [
        (4, d.fixed.form_size),
        (d.comm_pos + 10, d.fixed.frames),
        (d.ssnd_pos + 4, d.fixed.ssnd_size),
    ] {
        w.seek(SeekFrom::Start(pos))?;
        w.write_all(&(value as u32).to_be_bytes())?;
    }
    Ok(())
}

fn copy_n(src: &mut File, dst: &mut File, total: u64, progress: Progress) -> Result<()> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut done = 0u64;
    while done < total {
        let want = (total - done).min(COPY_BUF as u64) as usize;
        src.read_exact(&mut buf[..want])?;
        dst.write_all(&buf[..want])?;
        done += want as u64;
        if !progress(done, total) {
            return Err(Error::Cancelled);
        }
    }
    Ok(())
}

/// Write a repaired copy of `src` to `dst`. Refuses to overwrite `dst`.
pub fn repair_copy(src: &Path, dst: &Path, progress: Progress) -> Result<()> {
    let d = diagnose_broken(src)?;
    let mut input = File::open(src)?;
    let mut out = OpenOptions::new().write(true).create_new(true).open(dst)?;
    let copy_len = if d.extent_inferred {
        d.audio_start + d.audio_bytes
    } else {
        d.file_len
    };
    let result = (|| {
        copy_n(&mut input, &mut out, copy_len, progress)?;
        out.set_len(d.fixed.file_len)?; // adds the pad byte, if any
        patch_header(&mut out, &d)?;
        out.flush()?;
        Ok(())
    })();
    if result.is_err() {
        drop(out);
        let _ = std::fs::remove_file(dst);
    }
    result
}

/// Patch `path` in place. If `header_backup` is set, the original header bytes are saved to
/// `<path>.hdrbak` first.
pub fn repair_in_place(path: &Path, header_backup: bool) -> Result<()> {
    let d = diagnose_broken(path)?;
    if header_backup {
        let mut header = vec![0u8; d.audio_start as usize];
        File::open(path)?.read_exact(&mut header)?;
        let mut name = path.as_os_str().to_owned();
        name.push(".hdrbak");
        std::fs::write(name, header)?;
    }
    let mut f = OpenOptions::new().read(true).write(true).open(path)?;
    f.set_len(d.fixed.file_len)?; // drops a partial frame, adds the pad byte
    patch_header(&mut f, &d)?;
    f.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, Spec};
    use crate::{diagnose_path, Error, Health};
    use std::fs;

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.path().join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    fn audio_of(path: &Path) -> Vec<u8> {
        let d = diagnose_path(path).unwrap();
        let all = fs::read(path).unwrap();
        all[d.audio_start as usize..(d.audio_start + d.audio_bytes) as usize].to_vec()
    }

    fn no_progress() -> impl FnMut(u64, u64) -> bool {
        |_, _| true
    }

    #[test]
    fn copy_produces_a_healthy_file_with_identical_audio() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(5000);
        let src = write(&dir, "broken.aif", &fixtures::broken(spec, 3, 0));
        let dst = dir.path().join("fixed.aif");
        repair_copy(&src, &dst, &mut no_progress()).unwrap();
        let d = diagnose_path(&dst).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert_eq!(d.frames, 5000);
        assert_eq!(audio_of(&dst), fixtures::pcm(spec, 3));
    }

    #[test]
    fn copy_leaves_the_source_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = fixtures::broken(Spec::cd(100), 1, 3);
        let src = write(&dir, "broken.aif", &bytes);
        repair_copy(&src, &dir.path().join("fixed.aif"), &mut no_progress()).unwrap();
        assert_eq!(fs::read(&src).unwrap(), bytes);
    }

    #[test]
    fn copy_drops_a_partial_trailing_frame() {
        let dir = tempfile::tempdir().unwrap();
        let src = write(&dir, "broken.aif", &fixtures::broken(Spec::cd(100), 1, 3));
        let dst = dir.path().join("fixed.aif");
        repair_copy(&src, &dst, &mut no_progress()).unwrap();
        assert_eq!(fs::metadata(&dst).unwrap().len(), 54 + 400);
        assert_eq!(diagnose_path(&dst).unwrap().health, Health::Healthy);
    }

    #[test]
    fn copy_refuses_a_healthy_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = write(&dir, "ok.aif", &fixtures::healthy(Spec::cd(100), 1));
        let r = repair_copy(&src, &dir.path().join("out.aif"), &mut no_progress());
        assert!(matches!(r, Err(Error::NotBroken)));
        assert!(!dir.path().join("out.aif").exists());
    }

    #[test]
    fn copy_never_overwrites_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let src = write(&dir, "broken.aif", &fixtures::broken(Spec::cd(100), 1, 0));
        let dst = write(&dir, "fixed.aif", b"precious");
        assert!(repair_copy(&src, &dst, &mut no_progress()).is_err());
        assert_eq!(fs::read(&dst).unwrap(), b"precious");
    }

    #[test]
    fn cancelling_removes_the_partial_destination() {
        let dir = tempfile::tempdir().unwrap();
        let src = write(
            &dir,
            "broken.aif",
            &fixtures::broken(Spec::cd(400_000), 1, 0),
        );
        let dst = dir.path().join("fixed.aif");
        let r = repair_copy(&src, &dst, &mut |_, _| false);
        assert!(matches!(r, Err(Error::Cancelled)));
        assert!(!dst.exists());
    }

    #[test]
    fn progress_ends_at_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let src = write(
            &dir,
            "broken.aif",
            &fixtures::broken(Spec::cd(400_000), 1, 0),
        );
        let mut seen = Vec::new();
        repair_copy(&src, &dir.path().join("f.aif"), &mut |done, total| {
            seen.push((done, total));
            true
        })
        .unwrap();
        let (done, total) = *seen.last().unwrap();
        assert_eq!(done, total);
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn in_place_repair_fixes_the_header_and_trims_partial_frames() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(1000);
        let p = write(&dir, "broken.aif", &fixtures::broken(spec, 4, 3));
        repair_in_place(&p, false).unwrap();
        let d = diagnose_path(&p).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert_eq!(fs::metadata(&p).unwrap().len(), 54 + 4000);
        assert_eq!(audio_of(&p), fixtures::pcm(spec, 4));
    }

    #[test]
    fn in_place_repair_pads_odd_length_audio() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec {
            rate: 8000,
            channels: 1,
            bits: 8,
            frames: 5,
        };
        let p = write(&dir, "odd.aif", &fixtures::broken(spec, 0, 0));
        repair_in_place(&p, false).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().len(), 60);
        assert_eq!(diagnose_path(&p).unwrap().health, Health::Healthy);
    }

    #[test]
    fn in_place_repair_can_save_the_original_header() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = fixtures::broken(Spec::cd(100), 1, 0);
        let p = write(&dir, "broken.aif", &bytes);
        repair_in_place(&p, true).unwrap();
        let bak = dir.path().join("broken.aif.hdrbak");
        assert_eq!(fs::read(bak).unwrap(), &bytes[..54]);
    }

    #[test]
    fn in_place_repair_of_a_wrong_frame_count_keeps_the_id3_trailer() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = fixtures::healthy(Spec::cd(100), 1);
        bytes[22..26].copy_from_slice(&7u32.to_be_bytes());
        let p = write(&dir, "x.aif", &bytes);
        repair_in_place(&p, false).unwrap();
        let d = diagnose_path(&p).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert!(d.tags.is_some());
        assert_eq!(fs::metadata(&p).unwrap().len(), bytes.len() as u64);
    }

    #[test]
    fn in_place_repair_refuses_a_healthy_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "ok.aif", &fixtures::healthy(Spec::cd(10), 1));
        assert!(matches!(repair_in_place(&p, false), Err(Error::NotBroken)));
    }
}
