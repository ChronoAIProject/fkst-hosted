//! Package domain models, size-limit constants, and `NewPackage` validation.
//!
//! Conventions (load-bearing for downstream queries):
//! - Package identity is the `name` (`[A-Za-z0-9_-]+`, no version field),
//!   stored as the Mongo `_id` — mirroring the engine exactly.
//! - `files` is a BSON **array** of `{ path, content }` subdocuments (BSON
//!   keys cannot contain dots while file paths do); order is preserved.
//! - `composed_deps` is always an array (empty when absent, never null).
//! - Timestamps are `bson::DateTime` (millisecond UTC).

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// MongoDB collection holding package documents.
pub const PACKAGES_COLLECTION: &str = "packages";

/// Maximum number of files in a package (inclusive).
pub const MAX_FILES: usize = 256;
/// Maximum byte length of a single file path (inclusive).
pub const MAX_FILE_PATH_LEN: usize = 512;
/// Maximum byte size of a single file's content (inclusive; 1 MiB).
pub const MAX_FILE_CONTENT_BYTES: usize = 1_048_576;
/// Maximum total byte size of all file content (inclusive; 12 MiB).
///
/// MongoDB's hard BSON document limit is 16 MiB and the stored document also
/// carries every path (up to `MAX_FILES * MAX_FILE_PATH_LEN` = 128 KiB), the
/// array/subdocument BSON framing, `composed_deps`, `_id`, and timestamps.
/// Capping total content at 12 MiB keeps the whole document safely under the
/// limit with margin.
pub const MAX_TOTAL_CONTENT_BYTES: usize = 12_582_912;
/// Maximum number of `composed_deps` entries (inclusive).
pub const MAX_COMPOSED_DEPS: usize = 256;
/// Maximum byte length of a single trimmed `composed_deps` entry (inclusive).
pub const MAX_COMPOSED_DEP_LEN: usize = 256;

/// One file of a stored package. Files are an array (not a map) because BSON
/// keys cannot contain dots while file paths do. Array order is preserved
/// and round-tripped verbatim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageFile {
    pub path: String,
    pub content: String,
}

/// `packages` collection document: `_id` is the package name.
///
/// Effectively immutable in v1: `create` writes it once; reads never mutate.
/// `updated_at` equals `created_at` at creation and is reserved for a future
/// edit feature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Package {
    #[serde(rename = "_id")]
    pub name: String,
    pub files: Vec<PackageFile>,
    #[serde(default)]
    pub composed_deps: Vec<String>,
    /// User who owns this package. Omitted for legacy pre-auth docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    /// Organization this package belongs to. Omitted for personal packages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

/// Creation input, decoupled from the stored document: the stored document
/// owns its timestamps and is never trusted to carry client timestamps.
/// Unknown JSON fields are ignored (forgiving by design at this layer).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NewPackage {
    pub name: String,
    pub files: Vec<PackageFile>,
    #[serde(default)]
    pub composed_deps: Vec<String>,
}

/// Anchored package-name pattern (also the engine's identity rule).
fn name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_-]+$").expect("static name regex"))
}

/// True when `name` is a valid package name (fully matches `[A-Za-z0-9_-]+`).
///
/// This is the single identity rule shared by `NewPackage::validate` and any
/// edge that receives a package name outside a request body (e.g. a URL path
/// parameter): a valid name is always ASCII, URL-path-safe, and Mongo-`_id`
/// safe (no `/`, `.`, `$`, whitespace, or NUL can pass).
pub fn is_valid_name(name: &str) -> bool {
    name_regex().is_match(name)
}

/// Anchored department engine entry: `departments/<name>/main.lua`.
fn department_entry_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^departments/[A-Za-z0-9_-]+/main\.lua$").expect("static department regex")
    })
}

/// Anchored raiser engine entry: `raisers/<name>.lua`.
fn raiser_entry_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^raisers/[A-Za-z0-9_-]+\.lua$").expect("static raiser regex"))
}

/// True for a Windows-style drive prefix (`C:`, `d:`...).
fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Per-file path checks (validation sub-rules 3a-3f) in contractual order.
/// `index` is the file's position in the `files` array (error context only).
///
/// These checks are the authoritative defense for later on-disk
/// materialization: an unvalidated `..`, absolute path, backslash, or
/// control character could escape the package root.
fn validate_path(index: usize, path: &str) -> Result<(), String> {
    // 3a: empty path.
    if path.is_empty() {
        return Err(format!("empty file path (file index {index})"));
    }
    // 3b: byte-length limit.
    if path.len() > MAX_FILE_PATH_LEN {
        return Err(format!(
            "file path too long: {} bytes exceeds {MAX_FILE_PATH_LEN} (file index {index})",
            path.len()
        ));
    }
    // 3c: forward-slash-only separators.
    if path.contains('\\') {
        return Err(format!("invalid path separator: backslash in {path:?}"));
    }
    // 3d: no control characters (defends on-disk materialization).
    if path.chars().any(char::is_control) {
        return Err(format!(
            "invalid character in path: control character in {path:?}"
        ));
    }
    // 3e: no absolute paths (POSIX root or Windows drive prefix).
    if path.starts_with('/') || has_drive_prefix(path) {
        return Err(format!("absolute path not allowed: {path:?}"));
    }
    // 3f: no `..`, `.`, or empty segments (catches `a//b`, `./x`, `a/`).
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("unsafe path component in {path:?}"));
    }
    Ok(())
}

/// True when at least one file is an engine entry point: an anchored
/// `departments/<name>/main.lua` or `raisers/<name>.lua`. A path that merely
/// *contains* such a substring (e.g. `evil/departments/x/main.lua`) does not
/// count.
fn has_engine_entry(files: &[PackageFile]) -> bool {
    files.iter().any(|file| {
        department_entry_regex().is_match(&file.path) || raiser_entry_regex().is_match(&file.path)
    })
}

impl NewPackage {
    /// Pure validation, no I/O. `Err(String)` carries the human-readable
    /// reason for the FIRST violation, evaluated in the contractual total
    /// order 1 -> 2 -> 3a-g (per file, array order) -> 4 -> 5 -> 6 -> 7 -> 8.
    /// Each message starts with a stable, asserted-on prefix; the detail
    /// after the prefix may vary. Messages may include paths, sizes, and
    /// counts but never file content.
    pub fn validate(&self) -> Result<(), String> {
        // Rule 1: anchored name pattern (rejects empty/whitespace too).
        if !name_regex().is_match(&self.name) {
            return Err("invalid package name: must fully match [A-Za-z0-9_-]+".to_string());
        }
        // Rule 2: at least one file.
        if self.files.is_empty() {
            return Err("package has no files".to_string());
        }
        // Rule 3: per-file checks (array order; sub-rules a-f then g).
        for (index, file) in self.files.iter().enumerate() {
            validate_path(index, &file.path)?;
            // 3g: per-file content size.
            if file.content.len() > MAX_FILE_CONTENT_BYTES {
                return Err(format!(
                    "file content too large: {:?} is {} bytes (limit {MAX_FILE_CONTENT_BYTES})",
                    file.path,
                    file.content.len()
                ));
            }
        }
        // Rule 4: file count (after per-file checks, by contract).
        if self.files.len() > MAX_FILES {
            return Err(format!(
                "too many files: {} exceeds {MAX_FILES}",
                self.files.len()
            ));
        }
        // Rule 5: duplicate paths (case-sensitive, byte-exact).
        let mut seen = HashSet::with_capacity(self.files.len());
        for file in &self.files {
            if !seen.insert(file.path.as_str()) {
                return Err(format!("duplicate file path: {:?}", file.path));
            }
        }
        // Rule 6: at least one engine entry file.
        if !has_engine_entry(&self.files) {
            return Err(
                "no engine entry file: need departments/<name>/main.lua or raisers/<name>.lua"
                    .to_string(),
            );
        }
        // Rule 7: total content size.
        let total: usize = self.files.iter().map(|file| file.content.len()).sum();
        if total > MAX_TOTAL_CONTENT_BYTES {
            return Err(format!(
                "total content too large: {total} bytes exceeds {MAX_TOTAL_CONTENT_BYTES}"
            ));
        }
        // Rule 8: composed_deps count, then per-dep checks. Deps are
        // validated against their trimmed view but stored verbatim.
        if self.composed_deps.len() > MAX_COMPOSED_DEPS {
            return Err(format!(
                "too many composed_deps: {} exceeds {MAX_COMPOSED_DEPS}",
                self.composed_deps.len()
            ));
        }
        for (index, dep) in self.composed_deps.iter().enumerate() {
            let trimmed = dep.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "invalid composed_dep: blank entry at index {index}"
                ));
            }
            if trimmed.len() > MAX_COMPOSED_DEP_LEN {
                return Err(format!(
                    "invalid composed_dep: entry at index {index} exceeds \
                     {MAX_COMPOSED_DEP_LEN} bytes"
                ));
            }
            // The engine renders composed.deps one dep per line; an embedded
            // newline (or NUL) would forge extra dep lines.
            if dep.contains('\n') || dep.contains('\r') || dep.contains('\0') {
                return Err(format!(
                    "invalid composed_dep: newline or NUL in entry at index {index}"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bson::{doc, Bson};

    use super::*;

    fn file(path: &str, content: &str) -> PackageFile {
        PackageFile {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    fn entry() -> PackageFile {
        file("departments/router/main.lua", "return {}")
    }

    fn pkg(name: &str, files: Vec<PackageFile>) -> NewPackage {
        NewPackage {
            name: name.to_string(),
            files,
            composed_deps: Vec::new(),
        }
    }

    fn with_deps(deps: Vec<String>) -> NewPackage {
        NewPackage {
            name: "ok".to_string(),
            files: vec![entry()],
            composed_deps: deps,
        }
    }

    fn sample_package() -> Package {
        Package {
            name: "demo-package".to_string(),
            files: vec![entry(), file("lib/util.lua", "-- util")],
            composed_deps: vec!["base".to_string()],
            owner_user_id: None,
            org_id: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        }
    }

    // ---- serde shape -----------------------------------------------------

    #[test]
    fn package_round_trips_losslessly() {
        let package = sample_package();
        let raw = bson::to_document(&package).expect("serialize");
        let back: Package = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, package);
    }

    #[test]
    fn package_id_carries_the_name() {
        let raw = bson::to_document(&sample_package()).expect("serialize");
        assert_eq!(
            raw.get("_id").expect("_id present"),
            &Bson::String("demo-package".to_string())
        );
        assert!(!raw.contains_key("name"), "name must map onto _id only");
    }

    #[test]
    fn package_files_serialize_as_a_bson_array() {
        let raw = bson::to_document(&sample_package()).expect("serialize");
        match raw.get("files").expect("files present") {
            Bson::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected Bson::Array, got {other:?}"),
        }
    }

    #[test]
    fn package_empty_composed_deps_serialize_as_an_array_not_null() {
        let mut package = sample_package();
        package.composed_deps.clear();
        let raw = bson::to_document(&package).expect("serialize");
        assert_eq!(
            raw.get("composed_deps").expect("composed_deps present"),
            &Bson::Array(Vec::new())
        );
    }

    #[test]
    fn package_composed_deps_default_to_empty_on_deserialize() {
        let raw = doc! {
            "_id": "demo",
            "files": [{ "path": "core.lua", "content": "x" }],
            "created_at": bson::DateTime::from_millis(0),
            "updated_at": bson::DateTime::from_millis(0),
        };
        let package: Package = bson::from_document(raw).expect("deserialize");
        assert!(package.composed_deps.is_empty());
    }

    #[test]
    fn new_package_deserializes_without_composed_deps_and_ignores_unknown_fields() {
        let input: NewPackage = serde_json::from_str(
            r#"{"name":"demo","files":[{"path":"core.lua","content":"x"}],"extra":"ignored"}"#,
        )
        .expect("deserialize");
        assert_eq!(input.name, "demo");
        assert_eq!(input.files, vec![file("core.lua", "x")]);
        assert!(input.composed_deps.is_empty());
    }

    // ---- is_valid_name -----------------------------------------------------

    #[test]
    fn is_valid_name_accepts_engine_identity_names() {
        for name in ["demo", "My-Pkg_01", "a", "0", "_", "-", "A-Za-z0-9_-"] {
            assert!(is_valid_name(name), "must accept {name:?}");
        }
    }

    #[test]
    fn is_valid_name_rejects_invalid_names() {
        for name in [
            "a b", "a/b", "a.b", "a$b", "../x", "a\nb", "a\u{0}b", " demo", "demo ", "héllo",
        ] {
            assert!(!is_valid_name(name), "must reject {name:?}");
        }
    }

    #[test]
    fn is_valid_name_rejects_the_empty_string() {
        assert!(!is_valid_name(""));
    }

    #[test]
    fn is_valid_name_agrees_with_new_package_validation() {
        // Single source of truth: a name rejected here is rejected by
        // `NewPackage::validate` (rule 1) and vice versa.
        for name in ["ok-name", "a b", "", "a.b"] {
            let validated = pkg(name, vec![entry()]).validate().is_ok();
            assert_eq!(is_valid_name(name), validated, "diverged on {name:?}");
        }
    }

    // ---- validate: accepts -----------------------------------------------

    #[test]
    fn validate_accepts_minimal_department_only_package() {
        assert_eq!(pkg("demo", vec![entry()]).validate(), Ok(()));
    }

    #[test]
    fn validate_accepts_raiser_only_package() {
        let input = pkg("demo", vec![file("raisers/cron.lua", "return {}")]);
        assert_eq!(input.validate(), Ok(()));
    }

    #[test]
    fn validate_accepts_combined_package_with_core_and_deps() {
        let input = NewPackage {
            name: "My-Pkg_01".to_string(),
            files: vec![
                entry(),
                file("raisers/cron.lua", "return {}"),
                file("core.lua", "-- shared helpers"),
                // Dots inside a segment are allowed; only "." / ".." exact
                // segments are unsafe.
                file("lib/util.v2.lua", "-- dotted segment"),
            ],
            composed_deps: vec!["some-dep".to_string(), "another-dep".to_string()],
        };
        assert_eq!(input.validate(), Ok(()));
    }

    // ---- validate: reject matrix ------------------------------------------

    #[test]
    fn validate_rejects_each_violation_class_with_stable_prefix() {
        let long_path = "p".repeat(MAX_FILE_PATH_LEN + 1);
        let big_content = "x".repeat(MAX_FILE_CONTENT_BYTES + 1);

        let too_many_files = pkg(
            "ok",
            (0..=MAX_FILES)
                .map(|i| file(&format!("departments/d{i}/main.lua"), "x"))
                .collect(),
        );

        // 12 x 1 MiB == MAX_TOTAL_CONTENT_BYTES; one extra byte tips rule 7.
        let full = "x".repeat(MAX_FILE_CONTENT_BYTES);
        let mut total_over_files = vec![file("departments/x/main.lua", "")];
        for i in 0..12 {
            total_over_files.push(file(&format!("part{i}.lua"), &full));
        }
        total_over_files.push(file("one-more.lua", "x"));
        let total_over = pkg("ok", total_over_files);

        let too_many_deps = with_deps((0..=MAX_COMPOSED_DEPS).map(|i| format!("d{i}")).collect());

        let cases: Vec<(&str, NewPackage, &str)> = vec![
            // Rule 1: name regex.
            ("empty name", pkg("", vec![entry()]), "invalid package name"),
            (
                "whitespace name",
                pkg("  ", vec![entry()]),
                "invalid package name",
            ),
            (
                "name with space",
                pkg("a b", vec![entry()]),
                "invalid package name",
            ),
            (
                "name with slash",
                pkg("a/b", vec![entry()]),
                "invalid package name",
            ),
            (
                "name with dot",
                pkg("a.b", vec![entry()]),
                "invalid package name",
            ),
            // Rule 2: files non-empty.
            ("no files", pkg("ok", vec![]), "package has no files"),
            // Rule 3a: empty path.
            (
                "empty path",
                pkg("ok", vec![entry(), file("", "x")]),
                "empty file path",
            ),
            // Rule 3b: path length.
            (
                "over-long path",
                pkg("ok", vec![entry(), file(&long_path, "x")]),
                "file path too long",
            ),
            // Rule 3c: backslash.
            (
                "backslash path",
                pkg("ok", vec![entry(), file("dir\\file.lua", "x")]),
                "invalid path separator",
            ),
            // Rule 3d: control characters.
            (
                "NUL in path",
                pkg("ok", vec![entry(), file("a\u{0}b.lua", "x")]),
                "invalid character in path",
            ),
            (
                "control char in path",
                pkg("ok", vec![entry(), file("a\u{1f}b.lua", "x")]),
                "invalid character in path",
            ),
            // Rule 3e: absolute paths.
            (
                "absolute path",
                pkg("ok", vec![entry(), file("/etc/passwd", "x")]),
                "absolute path not allowed",
            ),
            (
                "drive prefix",
                pkg("ok", vec![entry(), file("C:/x", "x")]),
                "absolute path not allowed",
            ),
            // Rule 3f: unsafe segments.
            (
                "parent traversal",
                pkg("ok", vec![entry(), file("a/../b", "x")]),
                "unsafe path component",
            ),
            (
                "leading parent",
                pkg("ok", vec![entry(), file("../x", "x")]),
                "unsafe path component",
            ),
            (
                "double slash",
                pkg("ok", vec![entry(), file("a//b", "x")]),
                "unsafe path component",
            ),
            (
                "dot prefix",
                pkg("ok", vec![entry(), file("./x", "x")]),
                "unsafe path component",
            ),
            (
                "trailing slash",
                pkg("ok", vec![entry(), file("a/", "x")]),
                "unsafe path component",
            ),
            (
                "bare parent",
                pkg("ok", vec![entry(), file("..", "x")]),
                "unsafe path component",
            ),
            (
                "bare dot",
                pkg("ok", vec![entry(), file(".", "x")]),
                "unsafe path component",
            ),
            // Rule 3g: per-file content size.
            (
                "oversized file content",
                pkg("ok", vec![entry(), file("big.lua", &big_content)]),
                "file content too large",
            ),
            // Rule 4: file count.
            ("too many files", too_many_files, "too many files"),
            // Rule 5: duplicate paths.
            (
                "duplicate paths",
                pkg("ok", vec![entry(), entry()]),
                "duplicate file path",
            ),
            // Rule 6: engine entry.
            (
                "no engine entry",
                pkg("ok", vec![file("core.lua", "x")]),
                "no engine entry file",
            ),
            (
                "unanchored entry substring",
                pkg("ok", vec![file("evil/departments/x/main.lua", "x")]),
                "no engine entry file",
            ),
            (
                "dotted department segment is not an entry",
                pkg("ok", vec![file("departments/x.y/main.lua", "x")]),
                "no engine entry file",
            ),
            // Rule 7: total content size.
            (
                "oversized total content",
                total_over,
                "total content too large",
            ),
            // Rule 8: composed_deps.
            ("too many deps", too_many_deps, "too many composed_deps"),
            (
                "blank dep",
                with_deps(vec![String::new()]),
                "invalid composed_dep",
            ),
            (
                "whitespace dep",
                with_deps(vec!["   ".to_string()]),
                "invalid composed_dep",
            ),
            (
                "over-long dep",
                with_deps(vec!["d".repeat(MAX_COMPOSED_DEP_LEN + 1)]),
                "invalid composed_dep",
            ),
            (
                "dep with newline",
                with_deps(vec!["a\nb".to_string()]),
                "invalid composed_dep",
            ),
            (
                "dep with carriage return",
                with_deps(vec!["a\rb".to_string()]),
                "invalid composed_dep",
            ),
            (
                "dep with NUL",
                with_deps(vec!["a\u{0}b".to_string()]),
                "invalid composed_dep",
            ),
        ];

        for (label, input, expected_prefix) in cases {
            let err = input
                .validate()
                .expect_err(&format!("case {label:?} must be rejected"));
            assert!(
                err.starts_with(expected_prefix),
                "case {label:?}: expected prefix {expected_prefix:?}, got {err:?}"
            );
        }
    }

    // ---- validate: boundaries ----------------------------------------------

    #[test]
    fn validate_accepts_file_content_exactly_at_the_limit() {
        let exact = pkg(
            "ok",
            vec![
                entry(),
                file("big.lua", &"x".repeat(MAX_FILE_CONTENT_BYTES)),
            ],
        );
        assert_eq!(exact.validate(), Ok(()));
    }

    #[test]
    fn validate_total_content_boundary_is_inclusive() {
        // The cap divides evenly: 12 files of exactly MAX_FILE_CONTENT_BYTES
        // reach MAX_TOTAL_CONTENT_BYTES with an empty-content entry file.
        assert_eq!(MAX_TOTAL_CONTENT_BYTES, 12 * MAX_FILE_CONTENT_BYTES);
        let full = "x".repeat(MAX_FILE_CONTENT_BYTES);
        let mut files = vec![file("departments/x/main.lua", "")];
        for i in 0..12 {
            files.push(file(&format!("part{i}.lua"), &full));
        }
        assert_eq!(pkg("ok", files.clone()).validate(), Ok(()));

        files.push(file("one-more.lua", "x"));
        let err = pkg("ok", files).validate().expect_err("one byte over");
        assert!(err.starts_with("total content too large"), "got: {err}");
    }

    #[test]
    fn validate_file_count_boundary_is_inclusive() {
        let files: Vec<PackageFile> = (0..MAX_FILES)
            .map(|i| file(&format!("departments/d{i}/main.lua"), "x"))
            .collect();
        assert_eq!(pkg("ok", files.clone()).validate(), Ok(()));

        let mut over = files;
        over.push(file("extra.lua", "x"));
        let err = pkg("ok", over).validate().expect_err("one file over");
        assert!(err.starts_with("too many files"), "got: {err}");
    }

    #[test]
    fn validate_path_len_and_composed_dep_boundaries_are_inclusive() {
        let path = format!("dir/{}", "p".repeat(MAX_FILE_PATH_LEN - 4));
        assert_eq!(path.len(), MAX_FILE_PATH_LEN);
        assert_eq!(
            pkg("ok", vec![entry(), file(&path, "x")]).validate(),
            Ok(())
        );

        // MAX_COMPOSED_DEPS entries, each exactly MAX_COMPOSED_DEP_LEN bytes.
        let deps: Vec<String> = (0..MAX_COMPOSED_DEPS)
            .map(|i| format!("{i:0>width$}", width = MAX_COMPOSED_DEP_LEN))
            .collect();
        assert_eq!(with_deps(deps).validate(), Ok(()));
    }

    // ---- validate: first-violation total order -----------------------------

    #[test]
    fn first_violation_is_reported_in_total_order() {
        // Violates rule 1 (name), rule 3f (path), and rule 6 (no engine
        // entry) simultaneously: rule 1 must win.
        let err = pkg("a b", vec![file("../x", "y")])
            .validate()
            .expect_err("must be rejected");
        assert!(err.starts_with("invalid package name"), "got: {err}");
    }

    #[test]
    fn path_violation_is_reported_before_missing_engine_entry() {
        // Good name; violates rule 3f (path) and rule 6 (no engine entry):
        // the path violation must win.
        let err = pkg("ok", vec![file("a//b", "y")])
            .validate()
            .expect_err("must be rejected");
        assert!(err.starts_with("unsafe path component"), "got: {err}");
    }

    // ---- ownership field serde tests ----

    #[test]
    fn package_ownership_fields_are_omitted_when_absent() {
        let raw = bson::to_document(&sample_package()).expect("serialize");
        assert!(
            !raw.contains_key("owner_user_id"),
            "owner_user_id must be omitted when absent"
        );
        assert!(
            !raw.contains_key("org_id"),
            "org_id must be omitted when absent"
        );
    }

    #[test]
    fn package_ownership_fields_round_trip_when_set() {
        let mut package = sample_package();
        package.owner_user_id = Some("user-42".to_string());
        package.org_id = Some("org-1".to_string());
        let raw = bson::to_document(&package).expect("serialize");
        assert_eq!(
            raw.get_str("owner_user_id").expect("owner_user_id"),
            "user-42"
        );
        assert_eq!(raw.get_str("org_id").expect("org_id"), "org-1");
        let back: Package = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, package);
    }

    #[test]
    fn legacy_package_without_ownership_fields_still_deserializes() {
        let raw = doc! {
            "_id": "demo",
            "files": [{ "path": "departments/x/main.lua", "content": "return {}" }],
            "created_at": bson::DateTime::from_millis(0),
            "updated_at": bson::DateTime::from_millis(0),
        };
        let package: Package = bson::from_document(raw).expect("deserialize");
        assert_eq!(package.owner_user_id, None);
        assert_eq!(package.org_id, None);
    }
}
