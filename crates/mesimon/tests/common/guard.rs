//! The one test every area binary carries (T-1197).
//!
//! Cargo makes a test binary of each directory here that holds a `main.rs`,
//! and compiles only the files that `main.rs` names. A file dropped into an
//! area without its `mod` line, or into a directory with no `main.rs`, is
//! never built, so its tests would vanish without a red. This fails on one.
//! Each `main.rs` names this file by path; `common/mod.rs` does not, because
//! the relay's tests compile that module from their own repository.

#[test]
fn every_test_file_is_a_module() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut undeclared = Vec::new();
    let mut areas = 0;
    for area in std::fs::read_dir(&tests).unwrap().flatten() {
        let name = area.file_name().to_string_lossy().into_owned();
        if !area.path().is_dir() || name == "common" || name == "traces" {
            continue;
        }
        areas += 1;
        let main = std::fs::read_to_string(area.path().join("main.rs")).unwrap_or_default();
        for file in std::fs::read_dir(area.path()).unwrap().flatten() {
            let path = file.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let declared = main.lines().any(|line| line.trim() == format!("mod {stem};"));
            if path.extension().is_some_and(|e| e == "rs") && stem != "main" && !declared {
                undeclared.push(format!("{name}/{stem}.rs"));
            }
        }
    }
    assert!(areas > 0, "no area directory under {}", tests.display());
    assert!(
        undeclared.is_empty(),
        "never compiled, so their tests do not run: {undeclared:?}. Add `mod <file>;` to the \
         area's main.rs (a new area needs its own main.rs)"
    );
}
