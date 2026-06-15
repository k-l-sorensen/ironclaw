use std::{env, ffi::OsString};

use crate::{REBORN_PROFILE_ENV, RebornConfigError, RebornHome, RebornProfile};

/// Environment variable that enables Reborn learning behavior.
pub const LEARNING_ENABLED_ENV: &str = "IRONCLAW_LEARNING_ENABLED";

/// Fully resolved boot configuration for the standalone Reborn binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornBootConfig {
    home: RebornHome,
    profile: RebornProfile,
    learning_enabled: bool,
}

impl RebornBootConfig {
    pub fn new(home: RebornHome, profile: RebornProfile) -> Self {
        Self::new_with_learning_enabled(home, profile, false)
    }

    pub fn new_with_learning_enabled(
        home: RebornHome,
        profile: RebornProfile,
        learning_enabled: bool,
    ) -> Self {
        Self {
            home,
            profile,
            learning_enabled,
        }
    }

    pub fn resolve_from_env() -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env()?;
        let profile = RebornProfile::from_env_value(env::var_os(REBORN_PROFILE_ENV))?;
        let learning_enabled = learning_enabled_from_env_value(env::var_os(LEARNING_ENABLED_ENV))?;
        Ok(Self {
            home,
            profile,
            learning_enabled,
        })
    }

    pub fn resolve_from_env_parts(
        reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        Self::resolve_from_env_parts_with_learning_enabled(
            reborn_home,
            home,
            userprofile,
            profile,
            None,
        )
    }

    pub fn resolve_from_env_parts_with_learning_enabled(
        reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        profile: Option<OsString>,
        learning_enabled: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        let home = RebornHome::resolve_from_env_parts(reborn_home, home, userprofile)?;
        let profile = RebornProfile::from_env_value(profile)?;
        let learning_enabled = learning_enabled_from_env_value(learning_enabled)?;
        Ok(Self {
            home,
            profile,
            learning_enabled,
        })
    }

    pub fn home(&self) -> &RebornHome {
        &self.home
    }

    pub fn profile(&self) -> RebornProfile {
        self.profile
    }

    pub fn learning_enabled(&self) -> bool {
        self.learning_enabled
    }

    pub fn into_parts(self) -> (RebornHome, RebornProfile) {
        (self.home, self.profile)
    }

    pub fn into_parts_with_learning_enabled(self) -> (RebornHome, RebornProfile, bool) {
        (self.home, self.profile, self.learning_enabled)
    }
}

fn learning_enabled_from_env_value(value: Option<OsString>) -> Result<bool, RebornConfigError> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value = value.to_string_lossy();
    let trimmed = value.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" => Ok(true),
        "0" | "false" | "off" => Ok(false),
        _ => Err(RebornConfigError::InvalidBooleanEnv {
            name: LEARNING_ENABLED_ENV,
            value: value.into_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::REBORN_HOME_ENV;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    struct EnvGuard {
        key: &'static str,
        prior: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl Into<OsString>) -> Self {
            let prior = env::var_os(key);
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                env::set_var(key, value.into());
            }
            Self { key, prior }
        }

        fn unset(key: &'static str) -> Self {
            let prior = env::var_os(key);
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                env::remove_var(key);
            }
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: env mutation in tests is serialized through `env_lock()`.
            unsafe {
                match &self.prior {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn learning_enabled_env_unset_defaults_false() {
        let _lock = env_lock();
        let home = tempfile::tempdir().expect("tempdir");
        let _home = EnvGuard::set(REBORN_HOME_ENV, home.path().as_os_str());
        let _profile = EnvGuard::unset(REBORN_PROFILE_ENV);
        let _learning = EnvGuard::unset(LEARNING_ENABLED_ENV);

        let config = RebornBootConfig::resolve_from_env().expect("boot config");

        assert!(!config.learning_enabled());
    }

    #[test]
    fn learning_enabled_env_accepts_true_values() {
        let _lock = env_lock();
        let home = tempfile::tempdir().expect("tempdir");
        let _home = EnvGuard::set(REBORN_HOME_ENV, home.path().as_os_str());
        let _profile = EnvGuard::unset(REBORN_PROFILE_ENV);

        for value in ["1", "true", "TRUE", "on", "ON"] {
            let _learning = EnvGuard::set(LEARNING_ENABLED_ENV, value);
            let config = RebornBootConfig::resolve_from_env().expect("boot config");
            assert!(
                config.learning_enabled(),
                "{LEARNING_ENABLED_ENV}={value:?} must enable learning"
            );
        }
    }

    #[test]
    fn learning_enabled_env_accepts_false_values() {
        let _lock = env_lock();
        let home = tempfile::tempdir().expect("tempdir");
        let _home = EnvGuard::set(REBORN_HOME_ENV, home.path().as_os_str());
        let _profile = EnvGuard::unset(REBORN_PROFILE_ENV);

        for value in ["0", "false", "FALSE", "off", "OFF"] {
            let _learning = EnvGuard::set(LEARNING_ENABLED_ENV, value);
            let config = RebornBootConfig::resolve_from_env().expect("boot config");
            assert!(
                !config.learning_enabled(),
                "{LEARNING_ENABLED_ENV}={value:?} must disable learning"
            );
        }
    }
}
