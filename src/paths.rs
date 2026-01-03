//! Centralized path resolution for sshwarma
//!
//! Follows XDG Base Directory Specification with 12-factor env var overrides.
//!
//! ## Directory Layout
//!
//! ```text
//! ~/.local/share/sshwarma/     (XDG_DATA_HOME)
//! ├── sshwarma.db
//! └── host_key
//!
//! ~/.config/sshwarma/          (XDG_CONFIG_HOME)
//! ├── models.toml
//! ├── screen.lua
//! └── {user}.lua
//! ```
//!
//! ## Environment Variables
//!
//! | Variable | Description | Default |
//! |----------|-------------|---------|
//! | `SSHWARMA_DB` | Database path | `~/.local/share/sshwarma/sshwarma.db` |
//! | `SSHWARMA_HOST_KEY` | Host key path | `~/.local/share/sshwarma/host_key` |
//! | `SSHWARMA_MODELS_CONFIG` | Models config | `~/.config/sshwarma/models.toml` |

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::debug;

/// Get the XDG data directory for sshwarma
///
/// Priority: `XDG_DATA_HOME` > `~/.local/share`
pub fn data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("sshwarma");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/sshwarma");
    }

    // Last resort: current directory
    PathBuf::from(".")
}

/// Get the XDG config directory for sshwarma
///
/// Priority: `XDG_CONFIG_HOME` > `~/.config`
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("sshwarma");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/sshwarma");
    }

    // Last resort: current directory
    PathBuf::from(".")
}

/// Get the database path
///
/// Priority: `SSHWARMA_DB` env var > `data_dir()/sshwarma.db`
pub fn db_path() -> PathBuf {
    std::env::var("SSHWARMA_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_dir().join("sshwarma.db"))
}

/// Get the host key path
///
/// Priority: `SSHWARMA_HOST_KEY` env var > `data_dir()/host_key`
pub fn host_key_path() -> PathBuf {
    std::env::var("SSHWARMA_HOST_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_dir().join("host_key"))
}

/// Get the models config path
///
/// Priority: `SSHWARMA_MODELS_CONFIG` env var > `config_dir()/models.toml`
pub fn models_config_path() -> PathBuf {
    std::env::var("SSHWARMA_MODELS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| config_dir().join("models.toml"))
}

/// Ensure required directories exist
///
/// Creates `data_dir()` and `config_dir()` if they don't exist.
pub fn ensure_dirs() -> Result<()> {
    let data = data_dir();
    if !data.exists() {
        debug!("creating data directory: {}", data.display());
        std::fs::create_dir_all(&data)
            .with_context(|| format!("failed to create data directory: {}", data.display()))?;
    }

    let config = config_dir();
    if !config.exists() {
        debug!("creating config directory: {}", config.display());
        std::fs::create_dir_all(&config)
            .with_context(|| format!("failed to create config directory: {}", config.display()))?;
    }

    Ok(())
}

/// Log resolved paths for discoverability
pub fn log_paths() {
    use tracing::info;
    info!("📂 data directory: {}", data_dir().display());
    info!("📂 config directory: {}", config_dir().display());
    info!("📂 database: {}", db_path().display());
    info!("📂 host key: {}", host_key_path().display());
    info!("📂 models config: {}", models_config_path().display());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    // Mutex to serialize tests that modify env vars
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_path_env_vars() {
        env::remove_var("SSHWARMA_DB");
        env::remove_var("SSHWARMA_HOST_KEY");
        env::remove_var("SSHWARMA_MODELS_CONFIG");
        env::remove_var("XDG_DATA_HOME");
        env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn test_env_var_override_db() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_path_env_vars();
        env::set_var("SSHWARMA_DB", "/custom/path/test.db");
        assert_eq!(db_path(), PathBuf::from("/custom/path/test.db"));
        clear_path_env_vars();
    }

    #[test]
    fn test_env_var_override_host_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_path_env_vars();
        env::set_var("SSHWARMA_HOST_KEY", "/custom/host_key");
        assert_eq!(host_key_path(), PathBuf::from("/custom/host_key"));
        clear_path_env_vars();
    }

    #[test]
    fn test_env_var_override_models_config() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_path_env_vars();
        env::set_var("SSHWARMA_MODELS_CONFIG", "/custom/models.toml");
        assert_eq!(models_config_path(), PathBuf::from("/custom/models.toml"));
        clear_path_env_vars();
    }

    #[test]
    fn test_xdg_data_home_override() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_path_env_vars();
        env::set_var("XDG_DATA_HOME", "/xdg/data");
        assert_eq!(data_dir(), PathBuf::from("/xdg/data/sshwarma"));
        assert_eq!(db_path(), PathBuf::from("/xdg/data/sshwarma/sshwarma.db"));
        clear_path_env_vars();
    }

    #[test]
    fn test_xdg_config_home_override() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_path_env_vars();
        env::set_var("XDG_CONFIG_HOME", "/xdg/config");
        assert_eq!(config_dir(), PathBuf::from("/xdg/config/sshwarma"));
        assert_eq!(
            models_config_path(),
            PathBuf::from("/xdg/config/sshwarma/models.toml")
        );
        clear_path_env_vars();
    }
}
