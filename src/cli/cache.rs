//! `cache` subcommand — manage the Overpass and Overture disk caches.

use anyhow::Result;

use crate::cli::args::{CacheAction, CacheArgs};
use crate::cli::parse_cache_age;
use crate::{osm_cache, overture};

pub fn run_cache(args: &CacheArgs) -> Result<()> {
    match &args.action {
        CacheAction::List => {
            let overpass_entries = osm_cache::list_areas();
            let overture_entries = overture::list_overture_areas();

            if overpass_entries.is_empty() && overture_entries.is_empty() {
                println!("No cached entries.");
                return Ok(());
            }

            if !overpass_entries.is_empty() {
                println!("Overpass cache ({} entries):", overpass_entries.len());
                println!(
                    "  {:<10} {:<45} {:<10} AGE",
                    "TYPE", "BBOX (S,W,N,E)", "SIZE"
                );
                for entry in &overpass_entries {
                    let [s, w, n, e] = entry.bbox;
                    let bbox_str = format!("{s:.4},{w:.4},{n:.4},{e:.4}");
                    let size = format_size(entry.size_bytes);
                    let age = format_age(entry.created_at);
                    println!("  {:<10} {:<45} {:<10} {}", "overpass", bbox_str, size, age);
                }
                println!();
            }

            if !overture_entries.is_empty() {
                println!("Overture cache ({} entries):", overture_entries.len());
                println!(
                    "  {:<10} {:<45} {:<10} AGE",
                    "TYPE", "BBOX (S,W,N,E)", "SIZE"
                );
                for entry in &overture_entries {
                    let [s, w, n, e] = entry.bbox;
                    let bbox_str = format!("{s:.4},{w:.4},{n:.4},{e:.4}");
                    let size = format_size(entry.size_bytes);
                    let age = format_age(entry.created_at);
                    println!(
                        "  {:<10} {:<45} {:<10} {}",
                        entry.cli_type, bbox_str, size, age
                    );
                }
            }
            Ok(())
        }
        CacheAction::Stats => {
            let overpass_dir = osm_cache::cache_dir();
            let overture_dir = overture::overture_cache_dir();
            let overpass_entries = osm_cache::list_areas();
            let overture_entries = overture::list_overture_areas();

            let overpass_total: u64 = overpass_entries.iter().map(|e| e.size_bytes).sum();
            let overture_total: u64 = overture_entries.iter().map(|e| e.size_bytes).sum();

            println!("Cache Statistics");
            println!("────────────────────────────────────────");
            println!(
                "Overpass:  {} entries, {} total",
                overpass_entries.len(),
                format_size(overpass_total)
            );
            println!("  dir: {}", overpass_dir.display());
            println!(
                "Overture:  {} entries, {} total",
                overture_entries.len(),
                format_size(overture_total)
            );
            println!("  dir: {}", overture_dir.display());
            println!("────────────────────────────────────────");
            println!(
                "Total:     {} entries, {}",
                overpass_entries.len() + overture_entries.len(),
                format_size(overpass_total + overture_total)
            );
            Ok(())
        }
        CacheAction::Clear(clear_args) => {
            let min_age = match &clear_args.older_than {
                Some(s) => Some(parse_cache_age(s)?),
                None => None,
            };

            let clear_overpass = !clear_args.overture_only;
            let clear_overture = !clear_args.overpass_only;

            let mut total_deleted = 0usize;

            if clear_overpass {
                let n = osm_cache::clear(min_age)?;
                total_deleted += n;
                if n > 0 {
                    println!("Cleared {n} Overpass cache entries");
                }
            }
            if clear_overture {
                let n = overture::clear_overture_cache(min_age)?;
                total_deleted += n;
                if n > 0 {
                    println!("Cleared {n} Overture cache entries");
                }
            }

            if total_deleted == 0 {
                println!("No entries to clear.");
            }
            Ok(())
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_age(created_at: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(created_at);
    let secs = diff.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
