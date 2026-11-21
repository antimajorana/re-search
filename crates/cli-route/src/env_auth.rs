//! Billing safety for the subscription route.
//!
//! In non-interactive mode the Claude CLI uses an API key, a bearer token, an OAuth token
//! from the environment, or a third-party cloud provider in preference to the
//! subscription login. re:Search does not remove those variables (that would restrict how
//! the CLI authenticates); it refuses to start a turn while any of them is set, naming
//! the variables (never their values), unless the person opts in with
//! `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1`.

use std::ffi::OsString;

/// Opt-in variable: `1`, `true` or `yes` lets a turn start although environment
/// credentials are set.
pub const ALLOW_ENV_AUTH_VAR: &str = "RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH";

/// Names from `vars` that are set to a non-empty value.
pub fn detect(vars: &[&'static str], env: &dyn Fn(&str) -> Option<OsString>) -> Vec<&'static str> {
    vars.iter().copied().filter(|name| env(name).is_some_and(|v| !v.is_empty())).collect()
}

/// Whether the person set the opt-in ([`ALLOW_ENV_AUTH_VAR`] = `1`, `true` or `yes`). It also
/// lets a turn proceed when the CLI's `system/init` reports an `apiKeySource` other than
/// `"none"` (see [`crate::claude::check_auth_source`]).
pub fn opted_in(env: &dyn Fn(&str) -> Option<OsString>) -> bool {
    env(ALLOW_ENV_AUTH_VAR)
        .map(|v| v.to_string_lossy().trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

/// `Ok(names)` when a turn may start (`names` is empty, or lists the variables the person
/// explicitly allowed); `Err(message)` naming the variables otherwise.
pub fn check(vars: &[&'static str], env: &dyn Fn(&str) -> Option<OsString>) -> Result<Vec<&'static str>, String> {
    let found = detect(vars, env);
    if found.is_empty() || opted_in(env) {
        return Ok(found);
    }
    let (list, verb) = if found.len() == 1 { (found[0].to_string(), "is") } else { (found.join(", "), "are") };
    Err(format!(
        "refusing to start Claude Code: {list} {verb} set in re:Search's environment, and the Claude CLI \
         prefers it over your subscription login; this would bill an API key (or another provider) instead \
         of your Claude subscription. Unset it or use the API-key route, or set {ALLOW_ENV_AUTH_VAR}=1 to \
         proceed anyway."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::CLAUDE_ENV_AUTH_VARS;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(*v))).collect();
        move |n: &str| map.get(n).cloned()
    }

    #[test]
    fn clean_environment_passes() {
        assert_eq!(check(CLAUDE_ENV_AUTH_VARS, &env_of(&[("PATH", "x"), ("ANTHROPIC_BASE_URL", "y")])), Ok(vec![]));
        // Empty values count as unset.
        assert_eq!(check(CLAUDE_ENV_AUTH_VARS, &env_of(&[("ANTHROPIC_API_KEY", "")])), Ok(vec![]));
    }

    #[test]
    fn each_variable_is_detected_and_named_without_its_value() {
        for name in CLAUDE_ENV_AUTH_VARS {
            let err = check(CLAUDE_ENV_AUTH_VARS, &env_of(&[(name, "value-not-shown-123")])).unwrap_err();
            assert!(err.contains(name), "{err}");
            assert!(err.contains("subscription"), "{err}");
            assert!(err.contains(ALLOW_ENV_AUTH_VAR), "{err}");
            assert!(!err.contains("value-not-shown-123"), "{err}");
        }
        let err = check(CLAUDE_ENV_AUTH_VARS, &env_of(&[("ANTHROPIC_API_KEY", "a"), ("CLAUDE_CODE_USE_VERTEX", "1")]))
            .unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY, CLAUDE_CODE_USE_VERTEX are set"), "{err}");
    }

    #[test]
    fn explicit_opt_in_proceeds_and_reports_the_names() {
        for yes in ["1", "true", "YES"] {
            let env = env_of(&[("ANTHROPIC_AUTH_TOKEN", "t"), (ALLOW_ENV_AUTH_VAR, yes)]);
            assert_eq!(check(CLAUDE_ENV_AUTH_VARS, &env), Ok(vec!["ANTHROPIC_AUTH_TOKEN"]));
        }
        for no in ["0", "", "no", "false"] {
            let env = env_of(&[("ANTHROPIC_AUTH_TOKEN", "t"), (ALLOW_ENV_AUTH_VAR, no)]);
            assert!(check(CLAUDE_ENV_AUTH_VARS, &env).is_err(), "{no}");
        }
    }
}
