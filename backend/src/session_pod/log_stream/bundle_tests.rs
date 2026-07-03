//! Tests for the tar.gz bundle assembly. Split into a sibling file so `bundle.rs`
//! stays under the module-size cap. The archive is decoded back with `flate2` +
//! `tar` so the round trip (gzip valid + tree preserved) is asserted end to end.

use super::*;

use std::io::Read;

use flate2::read::GzDecoder;

/// Extract a `tar.gz` buffer into `(archive_path, contents)` pairs, normalizing any
/// leading `./` so a test can match on the tree-relative path.
fn extract(gz: &[u8]) -> Vec<(String, String)> {
    let decoder = GzDecoder::new(gz);
    let mut archive = tar::Archive::new(decoder);
    let mut out = Vec::new();
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let path = entry
            .path()
            .expect("path")
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        // Directories carry no bytes; only capture files.
        if entry.header().entry_type().is_file() {
            let mut contents = String::new();
            entry.read_to_string(&mut contents).expect("read entry");
            out.push((path, contents));
        }
    }
    out
}

fn find<'a>(entries: &'a [(String, String)], suffix: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|(p, _)| p.ends_with(suffix))
        .map(|(_, c)| c.as_str())
}

#[test]
fn tar_gz_dir_round_trips_the_tree_and_is_a_valid_gzip() {
    let dir = tempfile::tempdir().expect("dir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("fkst-hosted")).expect("mk hosted");
    std::fs::create_dir_all(root.join("fkst-substrate/framework")).expect("mk framework");
    std::fs::write(root.join("fkst-hosted/driver.log"), "driver line\n").expect("driver");
    std::fs::write(
        root.join("fkst-substrate/framework/supervise.log"),
        "supervise line\n",
    )
    .expect("supervise");
    std::fs::write(root.join("meta.json"), "{}\n").expect("meta");

    let gz = tar_gz_dir(root).expect("bundle");
    // A gzip stream starts with the 0x1f 0x8b magic; assert the header is present
    // (GzDecoder below fully validates the trailer/CRC by decoding).
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "gzip magic header");

    let entries = extract(&gz);
    assert_eq!(
        find(&entries, "fkst-hosted/driver.log"),
        Some("driver line\n")
    );
    assert_eq!(
        find(&entries, "fkst-substrate/framework/supervise.log"),
        Some("supervise line\n")
    );
    assert_eq!(find(&entries, "meta.json"), Some("{}\n"));
}

#[test]
fn tar_gz_dir_on_an_absent_root_is_still_a_valid_empty_gzip() {
    let dir = tempfile::tempdir().expect("dir");
    let missing = dir.path().join("never-created");
    let gz = tar_gz_dir(&missing).expect("empty bundle");
    // Decodes cleanly (valid gzip) and yields no file entries.
    let entries = extract(&gz);
    assert!(entries.is_empty(), "no entries: {entries:?}");
}
