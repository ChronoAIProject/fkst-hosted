//! Session progress journaling: capture engine progress signals, record them
//! durably (MongoDB `session_progress`), and surface them to GitHub (the
//! per-logical-run progress record file) so a redo on another pod can skip
//! already-completed work.
//!
//! Key derivation (the heart of the redo contract): every raised event gets a
//! stable, content-derived `idem_key`, and every logical run a content-derived
//! `run_key`, both identical whether produced by the original session or a
//! redo on a different pod. Correctness never depends on timestamps or on the
//! engine's LOCAL `once()` marks / `with_lock` / codex-permits.

pub mod parse;

use sha2::{Digest, Sha256};

use crate::packages::model::PackageFile;

/// ASCII Unit Separator: joins key-derivation parts (never appears in
/// validated package names or lowercase hex).
const US: u8 = 0x1f;

/// ASCII Record Separator: separates a file's path from its content inside
/// the package fingerprint.
const RS: u8 = 0x1e;

/// Domain tag versioning the package fingerprint derivation.
const PKG_FP_DOMAIN: &str = "fkst-pkg-fp@1";

/// Lowercase hex of a finished SHA-256 hasher.
fn finish_hex(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stable, content-derived idempotency key for one raised event (lowercase
/// sha256 hex, 64 chars). Identical across original and redo sessions:
/// `sha256(package_name || US || canonical_event_identity(event, pointers))`.
pub fn idem_key(package_name: &str, event_json: &serde_json::Value, pointers: &[String]) -> String {
    let identity = parse::canonical_event_identity(event_json, pointers);
    let mut hasher = Sha256::new();
    hasher.update(package_name.as_bytes());
    hasher.update([US]);
    hasher.update(identity.as_bytes());
    finish_hex(hasher)
}

/// Logical-run identity (lowercase sha256 hex, 64 chars):
/// `sha256(package_name || US || package_fingerprint)`. Inherently safe as a
/// GitHub journal file basename.
pub fn run_key(package_name: &str, package_fingerprint: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(package_name.as_bytes());
    hasher.update([US]);
    hasher.update(package_fingerprint.as_bytes());
    finish_hex(hasher)
}

/// Content fingerprint of a package (lowercase sha256 hex):
/// `sha256("fkst-pkg-fp@1" || US || join(US, [path || RS || content] sorted
/// by path) || US || join(US, composed_deps sorted))`. Any change to a file
/// path, file content, or dependency changes the fingerprint — and therefore
/// starts a fresh logical run.
pub fn package_fingerprint(files: &[PackageFile], composed_deps: &[String]) -> String {
    let mut sorted_files: Vec<&PackageFile> = files.iter().collect();
    sorted_files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut sorted_deps: Vec<&String> = composed_deps.iter().collect();
    sorted_deps.sort();

    let mut hasher = Sha256::new();
    hasher.update(PKG_FP_DOMAIN.as_bytes());
    hasher.update([US]);
    for (index, file) in sorted_files.iter().enumerate() {
        if index > 0 {
            hasher.update([US]);
        }
        hasher.update(file.path.as_bytes());
        hasher.update([RS]);
        hasher.update(file.content.as_bytes());
    }
    hasher.update([US]);
    for (index, dep) in sorted_deps.iter().enumerate() {
        if index > 0 {
            hasher.update([US]);
        }
        hasher.update(dep.as_bytes());
    }
    finish_hex(hasher)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file(path: &str, content: &str) -> PackageFile {
        PackageFile {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    fn pointers() -> Vec<String> {
        ["/department", "/source", "/name", "/corr"]
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    fn is_lower_hex_64(key: &str) -> bool {
        key.len() == 64
            && key
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    }

    // ---- idem_key ---------------------------------------------------------

    #[test]
    fn idem_key_is_deterministic_lowercase_hex_64() {
        let event = json!({"department":"d","source":"s","name":"n","corr":"c"});
        let a = idem_key("pkg", &event, &pointers());
        let b = idem_key("pkg", &event, &pointers());
        assert_eq!(a, b);
        assert!(is_lower_hex_64(&a), "got {a:?}");
    }

    #[test]
    fn idem_key_is_key_order_independent() {
        let a: serde_json::Value =
            serde_json::from_str(r#"{"department":"d","name":"n","source":"s","corr":"c"}"#)
                .expect("a");
        let b: serde_json::Value =
            serde_json::from_str(r#"{"corr":"c","source":"s","name":"n","department":"d"}"#)
                .expect("b");
        assert_eq!(
            idem_key("pkg", &a, &pointers()),
            idem_key("pkg", &b, &pointers())
        );
    }

    #[test]
    fn idem_key_changes_with_any_identity_pointer_or_package() {
        let base = json!({"department":"d","source":"s","name":"n","corr":"c"});
        let base_key = idem_key("pkg", &base, &pointers());
        for changed in [
            json!({"department":"X","source":"s","name":"n","corr":"c"}),
            json!({"department":"d","source":"X","name":"n","corr":"c"}),
            json!({"department":"d","source":"s","name":"X","corr":"c"}),
            json!({"department":"d","source":"s","name":"n","corr":"X"}),
        ] {
            assert_ne!(base_key, idem_key("pkg", &changed, &pointers()));
        }
        assert_ne!(base_key, idem_key("other-pkg", &base, &pointers()));
    }

    #[test]
    fn idem_key_all_missing_pointers_uses_a_stable_fallback() {
        let event = json!({"weird": true});
        let a = idem_key("pkg", &event, &pointers());
        let b = idem_key("pkg", &event, &pointers());
        assert_eq!(a, b);
        assert!(is_lower_hex_64(&a));
        assert_ne!(a, idem_key("pkg", &json!({"weird": false}), &pointers()));
    }

    // ---- run_key / package_fingerprint --------------------------------------

    #[test]
    fn run_key_is_deterministic_and_changes_with_inputs() {
        let fp = package_fingerprint(&[file("a.lua", "x")], &[]);
        let a = run_key("pkg", &fp);
        assert_eq!(a, run_key("pkg", &fp), "byte-for-byte identical");
        assert!(is_lower_hex_64(&a));
        assert_ne!(a, run_key("other", &fp));
        let fp2 = package_fingerprint(&[file("a.lua", "y")], &[]);
        assert_ne!(a, run_key("pkg", &fp2));
    }

    #[test]
    fn package_fingerprint_is_order_insensitive_for_files_and_deps() {
        let forward = package_fingerprint(
            &[file("a.lua", "1"), file("b.lua", "2")],
            &["dep-a".to_string(), "dep-b".to_string()],
        );
        let backward = package_fingerprint(
            &[file("b.lua", "2"), file("a.lua", "1")],
            &["dep-b".to_string(), "dep-a".to_string()],
        );
        assert_eq!(forward, backward);
    }

    #[test]
    fn package_fingerprint_changes_with_path_content_or_dep() {
        let base = package_fingerprint(&[file("a.lua", "1")], &["dep".to_string()]);
        assert_ne!(
            base,
            package_fingerprint(&[file("b.lua", "1")], &["dep".to_string()]),
            "path change"
        );
        assert_ne!(
            base,
            package_fingerprint(&[file("a.lua", "2")], &["dep".to_string()]),
            "content change"
        );
        assert_ne!(
            base,
            package_fingerprint(&[file("a.lua", "1")], &["other".to_string()]),
            "dep change"
        );
        assert_ne!(
            base,
            package_fingerprint(&[file("a.lua", "1")], &[]),
            "dep removal"
        );
    }

    #[test]
    fn package_fingerprint_separators_prevent_boundary_ambiguity() {
        // path/content boundary: "ab" + "c" vs "a" + "bc" must differ.
        assert_ne!(
            package_fingerprint(&[file("ab", "c")], &[]),
            package_fingerprint(&[file("a", "bc")], &[])
        );
        assert!(is_lower_hex_64(&package_fingerprint(&[], &[])));
    }
}
