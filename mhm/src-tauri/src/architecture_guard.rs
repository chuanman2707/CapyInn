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

/// The source with the bodies of inline `#[cfg(test)] mod … { … }` blocks removed.
///
/// Test fixtures seed tables with raw SQL; that is not the command layer owning
/// SQL, and counting it made this guard report roughly twice the real number. It
/// named `commands/invoices.rs` (10 test-only calls, zero in production) as the
/// largest remaining offender when that file was already correct.
///
/// Removes each block rather than truncating at the first one: `commands/pricing.rs`
/// has its test module in the *middle* of the file, with five more production
/// queries after it. Truncating there would have under-reported — a false clean,
/// which is the worse failure for a guard.
///
/// Relies on rustfmt: a file-level `mod` and its closing brace both sit at column
/// zero, so the block ends at the next line that is exactly `}`.
fn production_source(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        if lines[index] == "#[cfg(test)]" {
            let mut probe = index + 1;
            while probe < lines.len() && lines[probe].trim().is_empty() {
                probe += 1;
            }
            let opens_a_module = probe < lines.len()
                && lines[probe].starts_with("mod ")
                && lines[probe].ends_with('{');
            if opens_a_module {
                index = probe + 1;
                while index < lines.len() && lines[index] != "}" {
                    index += 1;
                }
                index += 1; // step past the closing brace
                continue;
            }
        }
        kept.push(lines[index]);
        index += 1;
    }

    kept.join("\n")
}

/// Command modules that still issue SQL in *production* code, largest first.
/// This is a ratchet, not a permission list: `docs/cleanup/` items E1–E3 are
/// about emptying it. The test fails both when an unlisted command module grows
/// SQL *and* when a listed one is cleaned up without being struck off, so the
/// list cannot quietly rot.
const COMMANDS_STILL_HOLDING_SQL: [&str; 0] = [];

#[test]
fn command_modules_only_hold_sql_where_the_cleanup_ratchet_still_allows_it() {
    let mut unexpected = Vec::new();
    let mut already_clean = Vec::new();

    for file in rust_files_in_layer("commands") {
        let name = relative(&file).replace('\\', "/");
        if is_test_module(&name) {
            continue;
        }
        let source = fs::read_to_string(&file).expect("read source file");
        let holds_sql = production_source(&source).contains("sqlx::query");
        let listed = COMMANDS_STILL_HOLDING_SQL.contains(&name.as_str());

        match (holds_sql, listed) {
            (true, false) => unexpected.push(name),
            (false, true) => already_clean.push(name),
            _ => {}
        }
    }

    assert!(
        unexpected.is_empty(),
        "commands orchestrate; they do not own SQL. Move reads into `queries/` \
         and writes behind a service or repository.\n\n{}",
        unexpected.join("\n")
    );
    assert!(
        already_clean.is_empty(),
        "these command modules no longer hold SQL — remove them from \
         COMMANDS_STILL_HOLDING_SQL so the ratchet keeps tightening.\n\n{}",
        already_clean.join("\n")
    );
}

#[test]
fn production_source_removes_inline_test_modules_wherever_they_sit() {
    let with_tests =
        "use sqlx::Pool;\n\nfn real() {}\n\n#[cfg(test)]\nmod tests {\n    sqlx::query(\"seed\");\n}\n";
    let production = production_source(with_tests);
    assert!(production.contains("fn real()"));
    assert!(!production.contains("sqlx::query"));

    // The `commands/pricing.rs` shape: production code *after* the test module.
    // Truncating at the first `#[cfg(test)]` reports this file as clean, which is
    // the worse failure for a guard. An earlier version of this helper did.
    let tests_in_the_middle = "fn early() {}\n\n#[cfg(test)]\nmod tests {\n    fn seed() {\n        let _ = 1;\n    }\n}\n\nfn late() {\n    sqlx::query(\"real\");\n}\n";
    let production = production_source(tests_in_the_middle);
    assert!(production.contains("fn early()"));
    assert!(production.contains("fn late()"));
    assert!(production.contains("sqlx::query"));

    // A blank line between the attribute and the module is the same shape.
    let spaced = "fn real() {}\n\n#[cfg(test)]\n\nmod tests {\n    sqlx::query(\"seed\");\n}\n";
    assert!(!production_source(spaced).contains("sqlx::query"));

    // `#[cfg(test)]` on a plain item must not swallow anything.
    let cfg_on_item =
        "#[cfg(test)]\npub fn helper() {}\n\nfn real() {\n    sqlx::query(\"real\");\n}\n";
    assert!(production_source(cfg_on_item).contains("sqlx::query"));
    assert!(production_source(cfg_on_item).contains("pub fn helper()"));

    // No test module at all: nothing is removed.
    let plain = "fn real() {\n    sqlx::query(\"real\");\n}";
    assert_eq!(production_source(plain), plain);
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
