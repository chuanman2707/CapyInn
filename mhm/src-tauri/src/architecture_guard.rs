//! Architecture fitness tests.
//!
//! `docs/architecture/core-pms-boundaries.md` defines the intended dependency
//! direction:
//!
//! ```text
//! write:  UI -> command -> service/lifecycle -> repository/transaction -> SQLite
//! read:   UI -> command -> query -> SQLite
//! ```
//!
//! `commands/` is the outermost boundary. Nothing further in may depend on it.
//! These tests read the source tree and fail if that direction is violated, so
//! a regression shows up as a red test instead of a review comment.

use std::fs;
use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/<layer>`, plus `src/<layer>.rs` if it exists.
fn rust_files_in_layer(layer: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let flat = src_dir().join(format!("{layer}.rs"));
    if flat.is_file() {
        files.push(flat);
    }
    collect_rust_files(&src_dir().join(layer), &mut files);
    files
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(src_dir())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Layers that sit *inside* the command boundary and must never import from it.
const INNER_LAYERS: [&str; 4] = ["domain", "queries", "repositories", "services"];

#[test]
fn inner_layers_do_not_import_the_command_layer() {
    let mut violations = Vec::new();

    for layer in INNER_LAYERS {
        let files = rust_files_in_layer(layer);
        assert!(
            !files.is_empty(),
            "layer `{layer}` has no Rust files — did it move? update INNER_LAYERS"
        );

        for file in files {
            let source = fs::read_to_string(&file).expect("read source file");
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.contains("crate::commands") || trimmed.contains("super::commands") {
                    violations.push(format!("{}:{}: {}", relative(&file), index + 1, trimmed));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "inner layers must not depend on `commands/` — commands are the outer \
         boundary and orchestrate inward, never the reverse.\n\
         Move the shared item down into `db`, `domain`, `models`, or `money` \
         instead of re-exporting it from `commands`.\n\n{}",
        violations.join("\n")
    );
}

/// Markers for *holding a connection or issuing SQL*, as opposed to merely
/// converting a `sqlx::Error` at a boundary, which is legitimate in any layer.
const DB_ACCESS_MARKERS: [&str; 5] = [
    "Pool<Sqlite>",
    "Transaction<",
    "SqliteConnection",
    "sqlx::query",
    "query_as",
];

#[test]
fn the_domain_layer_does_not_talk_to_sqlite() {
    let mut violations = Vec::new();

    for file in rust_files_in_layer("domain") {
        let name = relative(&file).replace('\\', "/");
        let source = fs::read_to_string(&file).expect("read source file");
        if let Some(marker) = DB_ACCESS_MARKERS
            .iter()
            .find(|marker| source.contains(**marker))
        {
            violations.push(format!("{name} (matched `{marker}`)"));
        }
    }

    assert!(
        violations.is_empty(),
        "the domain layer must hold pure business rules; database access belongs \
         behind `queries/`, `repositories/`, or `services/`.\n\n{}",
        violations.join("\n")
    );
}
