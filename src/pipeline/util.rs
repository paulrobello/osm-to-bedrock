//! Shared utility helpers for the conversion pipeline.
//!
//! Holds the on-disk zip helper used by both the server's conversion
//! endpoints and the CLI's final-packaging step, plus the small
//! deterministic-hash and closed-way predicates consumed by the renderer
//! and decoration modules.

use anyhow::Result;
use std::path::Path;

// ── Predicates shared across pipeline submodules ──────────────────────────────

/// Returns true if a way's first and last node ref are the same (closed polygon).
pub fn is_closed_way(refs: &[i64]) -> bool {
    refs.len() >= 4 && refs.first() == refs.last()
}

/// Deterministic hash for coordinate-based procedural generation.
pub fn coord_hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(374761393);
    h = h.wrapping_add((z as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

// ── Disk packaging ────────────────────────────────────────────────────────────

/// Zip a directory into a `.mcworld` file (which is just a zip archive).
pub fn zip_directory(dir: &Path, output_zip: &Path) -> Result<()> {
    use std::fs;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    // Count total files first for progress reporting.
    fn count_files(dir: &Path) -> usize {
        let mut n = 0;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    n += count_files(&path);
                } else {
                    n += 1;
                }
            }
        }
        n
    }

    let total_files = count_files(dir);
    log::info!("Zipping {total_files} files to {}", output_zip.display());

    let file = fs::File::create(output_zip)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut files_done: usize = 0;
    let mut last_logged_pct: usize = 0;

    // Walk the directory recursively
    fn add_dir_to_zip(
        zip_writer: &mut zip::ZipWriter<std::fs::File>,
        base: &Path,
        current: &Path,
        options: SimpleFileOptions,
        files_done: &mut usize,
        last_logged_pct: &mut usize,
        total_files: usize,
    ) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let rel = path.strip_prefix(base)?;
            let name = rel.to_string_lossy().to_string();

            if path.is_dir() {
                zip_writer.add_directory(format!("{name}/"), options)?;
                add_dir_to_zip(
                    zip_writer,
                    base,
                    &path,
                    options,
                    files_done,
                    last_logged_pct,
                    total_files,
                )?;
            } else {
                zip_writer.start_file(&name, options)?;
                let mut f = fs::File::open(&path)?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                zip_writer.write_all(&buf)?;

                *files_done += 1;
                if let Some(pct) = (*files_done)
                    .checked_mul(100)
                    .and_then(|v| v.checked_div(total_files))
                    && pct / 10 > *last_logged_pct / 10
                {
                    *last_logged_pct = pct;
                    log::info!("Zip progress: {pct}% ({}/{total_files} files)", *files_done);
                }
            }
        }
        Ok(())
    }

    add_dir_to_zip(
        &mut zip_writer,
        dir,
        dir,
        options,
        &mut files_done,
        &mut last_logged_pct,
        total_files,
    )?;
    zip_writer.finish()?;

    let zip_size = fs::metadata(output_zip).map(|m| m.len()).unwrap_or(0);
    log::info!("Zip complete: {}", format_bytes(zip_size));
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
