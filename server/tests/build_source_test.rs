//! The build source is a hand-kept allowlist.
//!
//! `flake.nix` copies a filtered subset of `server/` into the derivation: cargo sources,
//! plus `/assets`, `/golden`, `/fixtures` and `METRICS.md`. A test that reads a file
//! outside that list compiles and passes locally and then fails the *nix build* — which
//! is how the metrics reference broke `just ci` after it had passed every other check.
//!
//! This walks the crate for `include_str!` and checks each target against the same rules
//! the filter applies.
use std::path::{Path, PathBuf};

fn server_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The predicates from `flake.nix`'s `filter`, applied to a path relative to `server/`.
fn in_build_source(relative: &str) -> bool {
    let cargo_source = relative.ends_with(".rs")
        || relative.ends_with(".toml")
        || relative.ends_with("Cargo.lock");
    let explicitly_included = ["/assets", "/golden", "/fixtures", "METRICS.md"]
        .iter()
        .any(|fragment| format!("/{relative}").contains(fragment));
    cargo_source || explicitly_included
}

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the source tree is readable") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Every `include_str!("...")` / `include_bytes!("...")` target, resolved against the
/// file that reads it, as a path relative to the crate root.
fn included_files() -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources(&server_root().join("src"), &mut sources);
    sources.sort();

    let mut targets = Vec::new();
    for source in sources {
        let body = std::fs::read_to_string(&source).expect("a readable source file");
        for macro_name in ["include_str!", "include_bytes!"] {
            let mut rest = body.as_str();
            while let Some(at) = rest.find(macro_name) {
                rest = &rest[at + macro_name.len()..];
                let Some(open) = rest.find('"') else { break };
                let Some(close) = rest[open + 1..].find('"') else {
                    break;
                };
                let literal = &rest[open + 1..open + 1 + close];
                rest = &rest[open + 1 + close..];
                if literal.contains("env!") || literal.contains("concat!") {
                    continue; // computed at compile time; nothing to check here
                }
                let parent = source.parent().expect("a parent directory");
                let resolved = parent.join(literal);
                let relative = resolved
                    .strip_prefix(server_root())
                    .expect("an include inside the crate")
                    .to_string_lossy()
                    .to_string();
                targets.push(relative);
            }
        }
    }
    targets.sort();
    targets.dedup();
    targets
}

#[test]
fn every_file_a_test_reads_is_copied_into_the_build() {
    let targets = included_files();
    assert!(
        !targets.is_empty(),
        "the crate should read at least one file, or this check is vacuous"
    );
    for relative in &targets {
        assert!(
            server_root().join(relative).exists(),
            "src reads {relative}, which does not exist"
        );
        assert!(
            in_build_source(relative),
            "{relative} is read at compile time but is not copied into the derivation, so \
             the nix build will fail even though every other check passes. Add it to the \
             filter in flake.nix."
        );
    }
}

#[test]
fn the_allowlist_matches_the_filter_in_the_flake() {
    // Guard the guard: if the filter gains or loses a clause, this test is describing
    // something that is no longer true.
    let flake =
        std::fs::read_to_string(server_root().join("../flake.nix")).expect("a readable flake");
    for clause in [
        "filterCargoSources",
        "/assets",
        "/golden",
        "/fixtures",
        "METRICS.md",
    ] {
        assert!(
            flake.contains(clause),
            "flake.nix no longer mentions {clause}; keep this test and the filter together"
        );
    }
}

#[test]
fn the_rules_match_the_ones_the_filter_applies() {
    assert!(in_build_source("src/main.rs"));
    assert!(in_build_source("Cargo.toml"));
    assert!(in_build_source("Cargo.lock"));
    assert!(in_build_source("METRICS.md"));
    assert!(in_build_source("assets/eink-theme.tmTheme"));
    assert!(in_build_source("tests/fixtures/ocr/crop.png"));
    assert!(in_build_source("tests/golden/render/10-code-rust.html"));
    // Markdown at the crate root that is not the metrics reference is *not* copied: a
    // test reading it would pass here and fail in the build.
    assert!(!in_build_source("README.md"));
    assert!(!in_build_source("NOTES.md"));
}
