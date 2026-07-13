//! The per-session ENGINE-TUNABLE env pairs (issues #470/#471/#472): the
//! output locale, the operator-default rate pools, and the trigger author's
//! validated `### Engine Config` map, merged into the final `FKST_*` pairs the
//! session env carries. Split from [`super::session_launcher`] to keep that
//! module under the file-size cap and to give the tighten-merge its own
//! unit-testable home.
//!
//! Everything here rides the shared `session_env_pairs` (both backends) and
//! reaches the supervise child through the driver's base-env layer — the
//! reserved-key filter only guards the per-user environment store, and every
//! value arriving here was validated control-plane-side
//! ([`crate::goals::engine_config`] for user values, config load for operator
//! pools).

use std::collections::BTreeMap;

use crate::config::RatePool;

/// The engine's session output locale (the `t()` i18n SDK resolves
/// `locales/<value>.lua` by exact filename match, falling back to `en`).
const OUTPUT_LANG_ENV: &str = "FKST_OUTPUT_LANG";
/// Prefix rendering a rate pool as the engine's
/// `FKST_RATE_POOL_<NAME>=<burst>,<refill_per_minute>` definition.
const RATE_POOL_ENV_PREFIX: &str = "FKST_RATE_POOL_";
/// The engine's rate-pool ledger dir env. Pinned to a writable pod path whenever
/// any pool is defined: the engine's default is `~/.fkst/rate-pools` and its
/// `~`-expansion FAILS when HOME is unset — which a session container does not
/// guarantee — so injecting pools without pinning this would turn a working
/// session into a startup failure.
const RATE_POOL_ROOT_ENV: &str = "FKST_RATE_POOL_ROOT";
const RATE_POOL_ROOT_DIR: &str = "/var/run/fkst/rate-pools";

/// Render the engine-tunable env pairs for one session: the optional output
/// locale, then the TIGHTEN-MERGED engine config. Deterministic order (locale
/// first, then BTreeMap order) and — because the merge is map-level — free of
/// duplicate names by construction (a duplicate EnvVar would resolve last-wins
/// by kubelet accident, not by design).
///
/// The tighten-merge is the security boundary for rate pools: where an operator
/// default exists for a pool NAME, the effective value is the componentwise
/// `min(user, operator)` — a trigger author may only THROTTLE further, never
/// widen the operator's protection of the shared installation API budget. User
/// pools for NEW names pass through (adding a pool only ever throttles).
pub(crate) fn engine_tunables_env(
    output_lang: Option<&str>,
    user_config: &BTreeMap<String, String>,
    operator_pools: &BTreeMap<String, RatePool>,
) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    // The optional output locale — absent means ABSENT (no key), so a session
    // without the section renders byte-identical env to the pre-feature layout.
    if let Some(lang) = output_lang {
        pairs.push((OUTPUT_LANG_ENV.to_string(), lang.to_string()));
    }

    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    for (name, pool) in operator_pools {
        merged.insert(pool_key(name), render_pool(pool));
    }
    let mut has_pools = !operator_pools.is_empty();
    for (key, value) in user_config {
        match key.strip_prefix(RATE_POOL_ENV_PREFIX) {
            Some(name) => {
                has_pools = true;
                let effective = match operator_pools.get(name) {
                    Some(operator) => tighten_pool(value, operator),
                    None => value.clone(),
                };
                merged.insert(key.clone(), effective);
            }
            None => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    if has_pools {
        merged.insert(
            RATE_POOL_ROOT_ENV.to_string(),
            RATE_POOL_ROOT_DIR.to_string(),
        );
    }
    pairs.extend(merged);
    pairs
}

fn pool_key(name: &str) -> String {
    format!("{RATE_POOL_ENV_PREFIX}{name}")
}

fn render_pool(pool: &RatePool) -> String {
    format!("{},{}", pool.burst, pool.refill_per_minute)
}

/// Componentwise `min(user, operator)`. The user value arrives
/// parser-validated (`<burst>,<refill>`, both u64 ≥ 1); the impossible
/// malformed case falls back to the OPERATOR bound — never wider, and never a
/// panic inside the launch path.
fn tighten_pool(user_value: &str, operator: &RatePool) -> String {
    let parsed = user_value.split_once(',').and_then(|(burst, refill)| {
        Some((
            burst.trim().parse::<u64>().ok()?,
            refill.trim().parse::<u64>().ok()?,
        ))
    });
    match parsed {
        Some((burst, refill)) => format!(
            "{},{}",
            burst.min(operator.burst),
            refill.min(operator.refill_per_minute)
        ),
        None => render_pool(operator),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(burst: u64, refill: u64) -> RatePool {
        RatePool {
            burst,
            refill_per_minute: refill,
        }
    }

    #[test]
    fn empty_inputs_render_nothing() {
        let pairs = engine_tunables_env(None, &BTreeMap::new(), &BTreeMap::new());
        assert!(
            pairs.is_empty(),
            "no knobs set ⇒ the pre-feature env exactly"
        );
    }

    #[test]
    fn output_lang_renders_first_and_only_when_set() {
        let pairs = engine_tunables_env(Some("zh-CN"), &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(
            pairs,
            vec![("FKST_OUTPUT_LANG".to_string(), "zh-CN".to_string())]
        );
    }

    #[test]
    fn operator_pools_render_with_the_ledger_root() {
        let operator = BTreeMap::from([("GH".to_string(), pool(50, 50))]);
        let pairs = engine_tunables_env(None, &BTreeMap::new(), &operator);
        assert_eq!(
            pairs,
            vec![
                ("FKST_RATE_POOL_GH".to_string(), "50,50".to_string()),
                (
                    "FKST_RATE_POOL_ROOT".to_string(),
                    "/var/run/fkst/rate-pools".to_string()
                ),
            ]
        );
    }

    #[test]
    fn user_pools_tighten_only_against_operator_defaults() {
        let operator = BTreeMap::from([("GH".to_string(), pool(50, 50))]);
        // A user trying to WIDEN the operator pool is clamped componentwise…
        let user = BTreeMap::from([("FKST_RATE_POOL_GH".to_string(), "999,10".to_string())]);
        let pairs = engine_tunables_env(None, &user, &operator);
        let gh = pairs
            .iter()
            .find(|(k, _)| k == "FKST_RATE_POOL_GH")
            .unwrap();
        assert_eq!(
            gh.1, "50,10",
            "burst clamped to operator, refill kept (tighter)"
        );
        // …while a user pool for a NEW name passes through (it only throttles).
        let user = BTreeMap::from([("FKST_RATE_POOL_NPM".to_string(), "5,5".to_string())]);
        let pairs = engine_tunables_env(None, &user, &operator);
        let npm = pairs
            .iter()
            .find(|(k, _)| k == "FKST_RATE_POOL_NPM")
            .unwrap();
        assert_eq!(npm.1, "5,5");
    }

    #[test]
    fn user_pools_without_operator_defaults_still_pin_the_ledger_root() {
        let user = BTreeMap::from([("FKST_RATE_POOL_GH".to_string(), "5,5".to_string())]);
        let pairs = engine_tunables_env(None, &user, &BTreeMap::new());
        assert!(pairs.iter().any(|(k, _)| k == "FKST_RATE_POOL_ROOT"));
    }

    #[test]
    fn non_pool_user_keys_pass_through_and_names_stay_unique() {
        let operator = BTreeMap::from([("GH".to_string(), pool(50, 50))]);
        let user = BTreeMap::from([
            ("FKST_CODEX_PERMIT_SLOTS".to_string(), "8".to_string()),
            ("FKST_RATE_POOL_GH".to_string(), "10,10".to_string()),
        ]);
        let pairs = engine_tunables_env(Some("zh"), &user, &operator);
        let slots = pairs
            .iter()
            .find(|(k, _)| k == "FKST_CODEX_PERMIT_SLOTS")
            .unwrap();
        assert_eq!(slots.1, "8");
        let mut names: Vec<_> = pairs.iter().map(|(k, _)| k.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), pairs.len(), "env names must be unique");
    }
}
