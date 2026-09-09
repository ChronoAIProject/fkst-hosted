//! The `### Package Env` trigger section: per-package configuration.
//!
//! The trigger issue mixes two kinds of configuration. Session configuration
//! (name, branches, auto-merge, contributors) means the same thing whichever
//! packages run. Package configuration belongs to ONE package and is meaningless
//! without it — `### Packages`, `### Manifest`, and this section.
//!
//! Values here are grouped under `#### <package>` lines. `####` is deliberate:
//! [`crate::goals::section_parse::is_heading`] treats only `### ` as a section
//! boundary, so a `#### ` line stays INSIDE this section's block and we own its
//! meaning. That is also why the session half of the grouping is presentational
//! (two ignored divider headings in the template) rather than structural —
//! re-parenting the existing sections under `#### ` would make every live trigger
//! fail to parse and take the whole fleet down with it.
//!
//! The grammar mirrors `### Engine Config` so an author only learns one shape,
//! but the allowlist is inverted: engine config accepts a fixed set of platform
//! keys, whereas a package declares its own keys and the platform cannot know
//! them. We therefore validate the SHAPE strictly and deny only what would let a
//! session author reach a platform-owned variable.

use std::collections::BTreeMap;

use crate::error::AppError;
use crate::goals::section_parse::{non_empty_lines, strip_html_comments};

/// Per-package environment: package name → (key → value).
pub type PackageEnv = BTreeMap<String, BTreeMap<String, String>>;

/// Environment names the platform owns. A session author setting one of these
/// could redirect the session's identity, credentials, or routing, so they are
/// refused with a 422 rather than silently dropped later.
///
/// This list is asserted COMPLETE at runtime by a test that renders a fully
/// populated pod spec and checks every key it produces appears here — a static
/// list would drift the first time a new platform variable is added.
pub const PLATFORM_OWNED_SESSION_ENV: &[&str] = &[
    "FKST_GITHUB_AUTHORIZED_LOGINS",
    "FKST_GITHUB_BOT_LOGIN",
    "FKST_GITHUB_CLAIM_MODE",
    "FKST_GITHUB_PROXY_POLL_LABEL_PREFIX",
    "FKST_GITHUB_REPO",
    "FKST_GITHUB_WRITE",
    "FKST_SESSION_CREATOR",
    "FKST_SESSION_CREDS_DIR",
    "FKST_SESSION_DELIVERY_GRANTS",
    "FKST_SESSION_ID",
    "FKST_SESSION_PACKAGE_ENV_JSON",
    "FKST_SESSION_PACKAGE_ROOTS",
    "FKST_SESSION_WORK_LABEL",
    "FKST_SESSION_WORK_LABEL_MAP_JSON",
    "FKST_TRIGGER_ISSUE",
    crate::reconcile::work_labels::WORK_LABEL_NAMESPACE_ENV,
];

const MAX_BLOCKS: usize = 16;
const MAX_KEYS_PER_BLOCK: usize = 32;
const MAX_ENTRIES_TOTAL: usize = 64;
const MAX_SERIALIZED_BYTES: usize = 16 * 1024;
const MAX_KEY_BYTES: usize = 64;
const MAX_VALUE_BYTES: usize = 1024;
const MAX_PACKAGE_NAME_BYTES: usize = 64;

const HEADING: &str = "### Package Env";
const BLOCK_PREFIX: &str = "#### ";

fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PACKAGE_NAME_BYTES
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// `FKST_` + underscore-separated uppercase/digit groups. Anchored on `FKST_`
/// so a package key is always recognisable as fkst configuration in a pod's
/// environment, and so it can never collide with a shell or toolchain variable.
fn is_valid_key(key: &str) -> bool {
    if key.len() > MAX_KEY_BYTES || !key.starts_with("FKST_") {
        return false;
    }
    let rest = &key["FKST_".len()..];
    if rest.is_empty() {
        return false;
    }
    rest.split('_').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    })
}

/// Parse the `### Package Env` section body.
///
/// Returns an empty map for a missing or comment-only section, so the pristine
/// issue template — which carries the heading with only guidance comments under
/// it — configures nothing.
pub fn parse_package_env(block: &str) -> Result<PackageEnv, AppError> {
    let stripped = strip_html_comments(block);
    let mut env: PackageEnv = BTreeMap::new();
    // Which block first claimed each key, so a conflict can name both sides.
    let mut key_owner: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut total_entries = 0usize;

    for line in non_empty_lines(&stripped) {
        if let Some(name) = line.strip_prefix(BLOCK_PREFIX) {
            let name = name.trim();
            if !is_valid_package_name(name) {
                return Err(AppError::Unprocessable(format!(
                    "the `{HEADING}` section has an invalid package block {name:?}: expected a \
                     package name like `github-devloop` (letters, digits, `_`, `.`, `-`, \
                     1-{MAX_PACKAGE_NAME_BYTES} bytes)"
                )));
            }
            if env.contains_key(name) {
                return Err(AppError::Unprocessable(format!(
                    "the `{HEADING}` section declares the `#### {name}` block more than once"
                )));
            }
            if env.len() == MAX_BLOCKS {
                return Err(AppError::Unprocessable(format!(
                    "the `{HEADING}` section declares more than {MAX_BLOCKS} package blocks"
                )));
            }
            env.insert(name.to_string(), BTreeMap::new());
            current = Some(name.to_string());
            continue;
        }

        // A bare `KEY=value` before any `#### ` has no package to belong to.
        // Silently attaching it to some default would make the author think a
        // setting applied when it never could.
        let Some(package) = current.clone() else {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section has the line {line:?} before any `#### <package>` \
                 block: every setting must say which package it configures"
            )));
        };

        let (key, value) = line.split_once('=').ok_or_else(|| {
            AppError::Unprocessable(format!(
                "the `{HEADING}` section has a malformed line {line:?} under `#### {package}`: \
                 expected one KEY=value per line"
            ))
        })?;
        let (key, value) = (key.trim(), value.trim());

        if !is_valid_key(key) {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section sets an invalid key {key:?} under `#### {package}`: \
                 expected an uppercase name starting with `FKST_`, at most {MAX_KEY_BYTES} bytes"
            )));
        }
        if PLATFORM_OWNED_SESSION_ENV.contains(&key) {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section sets {key}, which the platform owns and sets for every \
                 session; remove it"
            )));
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section sets {key} to a value of {} bytes under \
                 `#### {package}`: the limit is {MAX_VALUE_BYTES}",
                value.len()
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section sets {key} to a value containing a control character \
                 under `#### {package}`"
            )));
        }

        // Cross-block conflict. The pod receives ONE flat environment, so the
        // same key under two packages cannot both apply — one would silently win.
        if let Some(owner) = key_owner.get(key) {
            if owner != &package {
                return Err(AppError::Unprocessable(format!(
                    "the `{HEADING}` section sets {key} under both `#### {owner}` and \
                     `#### {package}`: a key may be configured by only one package"
                )));
            }
        }

        let block = env
            .get_mut(&package)
            .expect("the current block was inserted when its heading was read");
        if block.len() == MAX_KEYS_PER_BLOCK && !block.contains_key(key) {
            return Err(AppError::Unprocessable(format!(
                "the `#### {package}` block sets more than {MAX_KEYS_PER_BLOCK} keys"
            )));
        }
        if block.insert(key.to_string(), value.to_string()).is_some() {
            return Err(AppError::Unprocessable(format!(
                "the `#### {package}` block sets {key} more than once"
            )));
        }
        key_owner.insert(key.to_string(), package);

        total_entries += 1;
        if total_entries > MAX_ENTRIES_TOTAL {
            return Err(AppError::Unprocessable(format!(
                "the `{HEADING}` section sets more than {MAX_ENTRIES_TOTAL} values in total"
            )));
        }
    }

    // Drop blocks an author declared but left empty, so `#### pkg` with only a
    // comment under it is indistinguishable from not writing the block at all.
    env.retain(|_, keys| !keys.is_empty());

    let serialized: usize = env
        .iter()
        .map(|(pkg, keys)| {
            pkg.len()
                + keys
                    .iter()
                    .map(|(k, v)| k.len() + v.len() + 2)
                    .sum::<usize>()
        })
        .sum();
    if serialized > MAX_SERIALIZED_BYTES {
        return Err(AppError::Unprocessable(format!(
            "the `{HEADING}` section is {serialized} bytes of configuration: the limit is \
             {MAX_SERIALIZED_BYTES}"
        )));
    }

    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(block: &str) -> String {
        match parse_package_env(block) {
            Err(AppError::Unprocessable(message)) => message,
            other => panic!("expected an Unprocessable error, got {other:?}"),
        }
    }

    #[test]
    fn parses_blocks_and_keys() {
        let env = parse_package_env(
            "#### github-devloop\nFKST_DEVLOOP_AUTO_REFINE_MAX=2\nFKST_DEVLOOP_ROLLUP_MERGE=manual\n\
             #### github-devloop-integration\nFKST_DEVLOOP_RED_WINDOW=45\n",
        )
        .expect("valid section");

        assert_eq!(env.len(), 2);
        assert_eq!(env["github-devloop"]["FKST_DEVLOOP_AUTO_REFINE_MAX"], "2");
        assert_eq!(env["github-devloop"]["FKST_DEVLOOP_ROLLUP_MERGE"], "manual");
        assert_eq!(
            env["github-devloop-integration"]["FKST_DEVLOOP_RED_WINDOW"],
            "45"
        );
    }

    #[test]
    fn a_comment_only_section_configures_nothing() {
        // The shipped template carries the heading with guidance under it. If
        // that parsed to anything, every session would inherit phantom config.
        let env = parse_package_env(
            "<!--\nOptional. Group settings under `#### <package>`.\n\
             #### github-devloop\nFKST_EXAMPLE=1\n-->\n",
        )
        .expect("comment-only section");
        assert!(env.is_empty());
    }

    #[test]
    fn an_empty_block_is_dropped() {
        let env = parse_package_env("#### github-devloop\n").expect("valid");
        assert!(env.is_empty());
    }

    #[test]
    fn a_setting_before_any_block_is_rejected() {
        let message = err("FKST_DEVLOOP_AUTO_REFINE_MAX=2\n#### github-devloop\n");
        assert!(
            message.contains("before any `#### <package>` block"),
            "{message}"
        );
    }

    #[test]
    fn a_duplicate_block_is_rejected() {
        let message = err("#### github-devloop\nFKST_A=1\n#### github-devloop\nFKST_B=2\n");
        assert!(message.contains("more than once"), "{message}");
    }

    #[test]
    fn a_duplicate_key_in_one_block_is_rejected() {
        let message = err("#### github-devloop\nFKST_A=1\nFKST_A=2\n");
        assert!(message.contains("FKST_A more than once"), "{message}");
    }

    #[test]
    fn the_same_key_under_two_packages_is_rejected() {
        // The pod gets one flat environment, so this cannot be resolved silently.
        let message = err("#### one\nFKST_SHARED=1\n#### two\nFKST_SHARED=2\n");
        assert!(message.contains("`#### one`"), "{message}");
        assert!(message.contains("`#### two`"), "{message}");
    }

    #[test]
    fn a_malformed_line_is_rejected() {
        let message = err("#### github-devloop\nnot a setting\n");
        assert!(
            message.contains("expected one KEY=value per line"),
            "{message}"
        );
    }

    #[test]
    fn an_invalid_key_is_rejected() {
        for key in ["lowercase", "NOPREFIX_A", "FKST_", "FKST__A", "FKST_a"] {
            let message = err(&format!("#### pkg\n{key}=1\n"));
            assert!(message.contains("invalid key"), "{key}: {message}");
        }
    }

    #[test]
    fn an_invalid_package_name_is_rejected() {
        let message = err("#### has spaces\nFKST_A=1\n");
        assert!(message.contains("invalid package block"), "{message}");
    }

    #[test]
    fn every_platform_owned_name_is_rejected() {
        for key in PLATFORM_OWNED_SESSION_ENV {
            let message = err(&format!("#### pkg\n{key}=x\n"));
            assert!(message.contains("the platform owns"), "{key}: {message}");
        }
    }

    /// Pinned by name, not only by the loop above: the namespace became load-bearing
    /// for artifact naming, so a session must never be able to declare one and thereby
    /// forge another namespace's identity in the artifacts it emits.
    #[test]
    fn the_work_label_namespace_cannot_be_set_by_a_trigger_author() {
        use crate::reconcile::work_labels::WORK_LABEL_NAMESPACE_ENV;

        assert!(PLATFORM_OWNED_SESSION_ENV.contains(&WORK_LABEL_NAMESPACE_ENV));
        let message = err(&format!(
            "#### pkg\n{WORK_LABEL_NAMESPACE_ENV}=other-tenant\n"
        ));
        assert!(message.contains("the platform owns"), "{message}");
    }

    #[test]
    fn an_oversized_value_is_rejected() {
        let message = err(&format!(
            "#### pkg\nFKST_BIG={}\n",
            "x".repeat(MAX_VALUE_BYTES + 1)
        ));
        assert!(message.contains("the limit is"), "{message}");
    }

    #[test]
    fn a_control_character_in_a_value_is_rejected() {
        let message = err("#### pkg\nFKST_A=one\u{7}two\n");
        assert!(message.contains("control character"), "{message}");
    }

    #[test]
    fn too_many_blocks_is_rejected() {
        let mut body = String::new();
        for i in 0..=MAX_BLOCKS {
            body.push_str(&format!("#### pkg-{i}\nFKST_KEY{i}=1\n"));
        }
        let message = err(&body);
        assert!(message.contains("package blocks"), "{message}");
    }

    #[test]
    fn too_many_keys_in_one_block_is_rejected() {
        let mut body = String::from("#### pkg\n");
        for i in 0..=MAX_KEYS_PER_BLOCK {
            body.push_str(&format!("FKST_KEY{i}=1\n"));
        }
        let message = err(&body);
        assert!(message.contains("keys"), "{message}");
    }
}

#[cfg(test)]
mod trigger_integration_tests {
    use crate::goals::trigger_parse::parse_trigger_issue_body;

    const MINIMAL: &str = "### Session Name\nsess\n\n### Packages\nowner/repo@main:packages/p\n";

    #[test]
    fn the_grouping_dividers_are_inert() {
        // The whole session-grouping half of this feature rests on this: the
        // dividers must change NOTHING a parser sees, or shipping them in the
        // template would alter how live triggers are read. Spelled literally
        // here because the template that ships them does not exist yet.
        let plain = parse_trigger_issue_body(MINIMAL).expect("minimal body");
        let grouped = parse_trigger_issue_body(
            "### Session Configuration\n\n### Session Name\nsess\n\n\
             ### Package Configuration\n\n### Packages\nowner/repo@main:packages/p\n",
        )
        .expect("grouped body");

        assert_eq!(plain.name, grouped.name);
        assert_eq!(plain.packages, grouped.packages);
        assert_eq!(plain.package_env, grouped.package_env);
    }

    #[test]
    fn a_trigger_without_the_section_has_empty_package_env() {
        let spec = parse_trigger_issue_body(MINIMAL).expect("minimal body");
        assert!(spec.package_env.is_empty());
    }

    #[test]
    fn package_env_reaches_the_spec() {
        let spec = parse_trigger_issue_body(&format!(
            "{MINIMAL}\n### Package Env\n#### github-devloop\nFKST_DEVLOOP_AUTO_REFINE_MAX=2\n"
        ))
        .expect("body with package env");
        assert_eq!(
            spec.package_env["github-devloop"]["FKST_DEVLOOP_AUTO_REFINE_MAX"],
            "2"
        );
    }

    #[test]
    fn an_invalid_package_env_fails_the_whole_trigger() {
        // Fail closed: a mis-typed setting must not be silently dropped, or the
        // author would believe a package was configured when it was not.
        let err = parse_trigger_issue_body(&format!("{MINIMAL}\n### Package Env\nFKST_ORPHAN=1\n"));
        assert!(err.is_err(), "an orphan setting must 422");
    }
}
