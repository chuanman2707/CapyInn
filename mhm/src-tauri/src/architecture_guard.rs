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

/// Test modules are exempt: driving a Tauri command is exactly how you
/// integration-test the outer boundary, and `services/booking/tests/` does so
/// deliberately. The rule constrains *production* dependency direction.
///
/// This only recognises whole-file test modules (`tests.rs`, anything under a
/// `tests/` directory). An inline `#[cfg(test)] mod tests` inside a production
/// file is still checked, which errs strict — the direction we want.
fn is_test_module(relative_path: &str) -> bool {
    relative_path.ends_with("tests.rs")
        || relative_path.contains("/tests/")
        || relative_path.starts_with("tests/")
}

/// Finds imports of the command layer, including ones nested inside a
/// `use crate::{ ... }` block.
///
/// Matching the literal `crate::commands` alone is not enough: rustfmt splits
/// `use crate::{ commands::reservations, models::… }` across lines, leaving a
/// bare `commands::reservations,` that contains no `crate::` prefix at all. An
/// earlier version of this guard missed exactly that case and reported a clean
/// tree while `services/booking/tests.rs` imported the command layer.
///
/// Relies on rustfmt's layout: a `use crate::{` block closes with `};` at the
/// start of a line.
fn command_layer_imports(source: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    let mut inside_crate_use_block = false;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let line_number = index + 1;

        if trimmed.starts_with("//") {
            continue;
        }

        if trimmed.contains("crate::commands")
            || trimmed.contains("super::commands")
            || trimmed.contains("crate::{commands")
        {
            hits.push((line_number, trimmed.to_string()));
            continue;
        }

        if trimmed.starts_with("use crate::{") {
            inside_crate_use_block = !trimmed.ends_with("};");
            continue;
        }

        if inside_crate_use_block {
            if trimmed.starts_with("};") {
                inside_crate_use_block = false;
            } else if trimmed.starts_with("commands::") || trimmed == "commands," {
                hits.push((line_number, trimmed.to_string()));
            }
        }
    }

    hits
}

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
            let name = relative(&file).replace('\\', "/");
            if is_test_module(&name) {
                continue;
            }
            let source = fs::read_to_string(&file).expect("read source file");
            for (line_number, line) in command_layer_imports(&source) {
                violations.push(format!("{name}:{line_number}: {line}"));
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

#[test]
fn command_layer_imports_sees_through_a_nested_use_block() {
    // The exact shape that defeated the first version of this guard.
    let nested = "use crate::{\n    commands::reservations,\n    models::Booking,\n};\n";
    assert_eq!(
        command_layer_imports(nested)
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>(),
        vec!["commands::reservations,"]
    );

    for source in [
        "use crate::commands::mod_thing;\n",
        "use super::commands::thing;\n",
        "use crate::{commands::reservations, models::Booking};\n",
    ] {
        assert_eq!(
            command_layer_imports(source).len(),
            1,
            "should have flagged: {source}"
        );
    }

    for clean in [
        "use crate::{\n    models::Booking,\n    money::MoneyVnd,\n};\n",
        "// use crate::commands::thing;\n",
        "use crate::db::row::get_money_vnd;\n",
        // `commands` nested under a *different* root is not the command layer.
        "use other::{\n    commands::thing,\n};\n",
    ] {
        assert!(
            command_layer_imports(clean).is_empty(),
            "should have been clean: {clean}"
        );
    }
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
