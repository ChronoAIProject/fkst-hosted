//! Pure zip-archive-to-`NewPackage` converter. Never touches disk.
//!
//! Given raw zip bytes and a package name, this module extracts every
//! non-directory entry, enforces size/count/encoding caps, parses an optional
//! root `composed.deps`, and delegates final validation to `NewPackage::validate`
//! (the single authoritative gate). Memory usage stays bounded at approximately
//! `MAX_TOTAL_CONTENT_BYTES` (12 MiB) decoded content regardless of the zip's
//! compression ratio (zip-bomb guard).

use std::io::{Cursor, Read};

use super::model::{
    NewPackage, PackageFile, MAX_COMPOSED_DEP_LEN, MAX_FILES, MAX_FILE_CONTENT_BYTES,
    MAX_TOTAL_CONTENT_BYTES,
};

/// Root-level composed.deps file name (case-sensitive).
const COMPOSED_DEPS_FILENAME: &str = "composed.deps";
/// Root-level host-owned file that must not appear in user archives.
const FKST_ENV_FILENAME: &str = "fkst.env";

/// Parse a zip archive into a [`NewPackage`]. Pure function; no I/O beyond
/// reading from the provided byte slice through the `zip` crate.
///
/// Returns `Err(String)` with a human-readable reason on any violation:
/// - zip structure errors
/// - entry count > `MAX_FILES + 1` (the +1 allows a root `composed.deps`)
/// - encrypted entries
/// - per-file content > `MAX_FILE_CONTENT_BYTES`
/// - total decoded content > `MAX_TOTAL_CONTENT_BYTES`
/// - non-UTF-8 content
/// - root `fkst.env` present
/// - any violation caught by `NewPackage::validate`
pub fn package_from_zip(name: &str, bytes: &[u8]) -> Result<NewPackage, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("invalid zip archive: {e}"))?;

    // Early rejection: entry count over the allowance. The +1 accounts for
    // a possible root `composed.deps` which does not count toward the file
    // cap (it is parsed into composed_deps, not stored as a file).
    if archive.len() > MAX_FILES + 1 {
        return Err(format!(
            "too many entries: {} exceeds {}",
            archive.len(),
            MAX_FILES
        ));
    }

    let mut files: Vec<PackageFile> = Vec::new();
    let mut composed_deps: Vec<String> = Vec::new();
    let mut total_decoded: usize = 0;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry {index}: {e}"))?;

        let entry_name = entry.name().to_string();

        // Skip directory entries entirely.
        if entry.is_dir() {
            continue;
        }

        // Reject encrypted entries.
        if entry.encrypted() {
            return Err(format!("encrypted zip entry not allowed: {entry_name:?}"));
        }

        // Root `composed.deps`: parse into composed_deps, not stored as a file.
        if entry_name == COMPOSED_DEPS_FILENAME {
            let mut raw = String::new();
            entry
                .read_to_string(&mut raw)
                .map_err(|e| format!("failed to read composed.deps: {e}"))?;

            // Cap the composed.deps content against the running total.
            total_decoded = check_total(total_decoded, raw.len(), &entry_name)?;

            for (line_idx, line) in raw.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.len() > MAX_COMPOSED_DEP_LEN {
                    return Err(format!(
                        "composed.dep entry at line {} exceeds {} bytes",
                        line_idx + 1,
                        MAX_COMPOSED_DEP_LEN
                    ));
                }
                composed_deps.push(trimmed.to_string());
            }
            continue;
        }

        // Root `fkst.env`: host-owned file, rejected.
        if entry_name == FKST_ENV_FILENAME {
            return Err(format!(
                "host-owned file {FKST_ENV_FILENAME} not allowed in archive"
            ));
        }

        // Bounded read: cap per-file decoded size at MAX_FILE_CONTENT_BYTES + 1
        // (one byte over triggers rejection without buffering more).
        let mut buf = Vec::new();
        entry
            .take((MAX_FILE_CONTENT_BYTES + 1) as u64)
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed to read zip entry {entry_name:?}: {e}"))?;

        if buf.len() > MAX_FILE_CONTENT_BYTES {
            return Err(format!(
                "file content too large: {:?} exceeds {} bytes",
                entry_name, MAX_FILE_CONTENT_BYTES
            ));
        }

        // Zip-bomb guard: running decoded total must stay under cap.
        total_decoded = check_total(total_decoded, buf.len(), &entry_name)?;

        // Content must be valid UTF-8.
        let content = String::from_utf8(buf)
            .map_err(|_| format!("file content not valid UTF-8: {entry_name:?}"))?;

        files.push(PackageFile {
            path: entry_name,
            content,
        });
    }

    let pkg = NewPackage {
        name: name.to_string(),
        files,
        composed_deps,
    };

    // Single authoritative validation gate.
    pkg.validate()?;

    Ok(pkg)
}

/// Check and update the running decoded total. Returns `Err` if the cap is
/// exceeded, otherwise returns the new total.
fn check_total(current: usize, additional: usize, entry_name: &str) -> Result<usize, String> {
    let new_total = current + additional;
    if new_total > MAX_TOTAL_CONTENT_BYTES {
        return Err(format!(
            "total decoded content exceeds {} bytes at entry {:?}",
            MAX_TOTAL_CONTENT_BYTES, entry_name
        ));
    }
    Ok(new_total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: build a zip in memory with the given entries.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).expect("start_file");
            writer.write_all(data).expect("write");
        }
        writer.finish().expect("finish");
        buf.into_inner()
    }

    /// Helper: build a zip in memory with owned entries (avoids lifetime issues).
    fn build_zip_owned(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer
                .start_file(name.as_str(), options)
                .expect("start_file");
            writer.write_all(data).expect("write");
        }
        writer.finish().expect("finish");
        buf.into_inner()
    }

    fn lua_entry() -> (&'static str, &'static [u8]) {
        ("departments/x/main.lua", b"return {}")
    }

    #[test]
    fn basic_zip_produces_valid_package() {
        let bytes = build_zip(&[lua_entry(), ("lib/util.lua", b"-- util")]);
        let pkg = package_from_zip("test-pkg", &bytes).expect("valid");
        assert_eq!(pkg.name, "test-pkg");
        assert_eq!(pkg.files.len(), 2);
        assert!(pkg.composed_deps.is_empty());
    }

    #[test]
    fn composed_deps_are_parsed_not_stored() {
        let bytes = build_zip(&[lua_entry(), ("composed.deps", b"dep-a\ndep-b\n")]);
        let pkg = package_from_zip("test-pkg", &bytes).expect("valid");
        assert_eq!(pkg.composed_deps, vec!["dep-a", "dep-b"]);
        assert!(
            !pkg.files.iter().any(|f| f.path == "composed.deps"),
            "composed.deps must not appear in files"
        );
    }

    #[test]
    fn root_fkst_env_is_rejected() {
        let bytes = build_zip(&[lua_entry(), ("fkst.env", b"HOST_VAR=x")]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("fkst.env"), "got: {err}");
    }

    #[test]
    fn non_utf8_content_is_rejected() {
        let bytes = build_zip(&[lua_entry(), ("bad.lua", &[0xff, 0xfe])]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("not valid UTF-8"), "got: {err}");
    }

    #[test]
    fn too_many_entries_rejected() {
        // MAX_FILES + 2 entries: one over the allowance of MAX_FILES + 1.
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..=MAX_FILES + 1 {
            entries.push((format!("departments/d{i}/main.lua"), vec![b'x']));
        }
        let bytes = build_zip_owned(&entries);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.starts_with("too many entries"), "got: {err}");
    }

    #[test]
    fn oversized_single_file_rejected() {
        let big = vec![b'x'; MAX_FILE_CONTENT_BYTES + 1];
        let bytes = build_zip(&[lua_entry(), ("big.lua", &big)]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.starts_with("file content too large"), "got: {err}");
    }

    #[test]
    fn total_decoded_size_cap_enforced() {
        // MAX_FILE_CONTENT_BYTES per file, fill up to cap then add one more.
        let full = vec![b'x'; MAX_FILE_CONTENT_BYTES];
        let mut entries: Vec<(String, Vec<u8>)> =
            vec![("departments/x/main.lua".to_string(), vec![])];
        // 12 x MAX_FILE_CONTENT_BYTES = MAX_TOTAL_CONTENT_BYTES; already at cap.
        for i in 0..12 {
            entries.push((format!("part{i}.lua"), full.clone()));
        }
        // One more byte tips over.
        entries.push(("one-more.lua".to_string(), vec![b'x']));
        let bytes = build_zip_owned(&entries);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("total decoded content exceeds"), "got: {err}");
    }

    #[test]
    fn zip_slip_paths_rejected_by_validate() {
        let bytes = build_zip(&[lua_entry(), ("../evil.lua", b"x")]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("unsafe path component"), "got: {err}");
    }

    #[test]
    fn absolute_path_rejected_by_validate() {
        let bytes = build_zip(&[lua_entry(), ("/etc/passwd", b"x")]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("absolute path"), "got: {err}");
    }

    #[test]
    fn empty_zip_rejected_as_no_files() {
        let bytes = build_zip(&[]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("no files"), "got: {err}");
    }

    #[test]
    fn directories_only_rejected_as_no_files() {
        let mut buf = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .add_directory("subdir/", options)
            .expect("add_directory");
        writer.finish().expect("finish");
        let bytes = buf.into_inner();
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("no files"), "got: {err}");
    }

    #[test]
    fn blank_lines_in_composed_deps_are_dropped() {
        let bytes = build_zip(&[lua_entry(), ("composed.deps", b"\n  dep-a  \n\n dep-b \n")]);
        let pkg = package_from_zip("test-pkg", &bytes).expect("valid");
        assert_eq!(pkg.composed_deps, vec!["dep-a", "dep-b"]);
    }

    #[test]
    fn overlong_composed_dep_entry_rejected() {
        let long_dep = "x".repeat(MAX_COMPOSED_DEP_LEN + 1);
        let bytes = build_zip(&[lua_entry(), ("composed.deps", long_dep.as_bytes())]);
        let err = package_from_zip("test-pkg", &bytes).expect_err("must reject");
        assert!(err.contains("composed.dep entry"), "got: {err}");
    }

    #[test]
    fn invalid_package_name_rejected() {
        let bytes = build_zip(&[lua_entry()]);
        let err = package_from_zip("bad name", &bytes).expect_err("must reject");
        assert!(err.starts_with("invalid package name"), "got: {err}");
    }
}
