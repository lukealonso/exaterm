use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default, alias = "remotes")]
    pub hosts: Vec<RememberedHost>,
    #[serde(default)]
    pub terminal: TerminalConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hosts: Vec::new(),
            terminal: TerminalConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn normalized(mut self) -> Self {
        self.hosts.retain(|host| !host.target.trim().is_empty());
        for host in &mut self.hosts {
            host.target = host.target.trim().to_string();
        }
        self.terminal = self.terminal.normalized();
        self
    }

    pub fn remove_host(&mut self, target: &str) -> bool {
        let target = target.trim();
        let previous_len = self.hosts.len();
        self.hosts.retain(|host| host.target != target);
        self.hosts.len() != previous_len
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedHost {
    pub target: String,
    pub last_used_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalConfig {
    #[serde(default)]
    pub audible_bell: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            audible_bell: false,
        }
    }
}

impl TerminalConfig {
    pub fn normalized(self) -> Self {
        self
    }
}

pub fn load_app_config() -> AppConfig {
    if let Some(path) = app_config_path() {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<AppConfig>(&raw) {
                return config.normalized();
            }
        }
    }

    let config = load_legacy_launcher_config()
        .unwrap_or_default()
        .normalized();
    if !config.hosts.is_empty() {
        let _ = save_app_config(&config);
    }
    config
}

pub fn save_app_config(config: &AppConfig) -> Result<(), String> {
    let path = app_config_path().ok_or("could not determine Exaterm config path")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }
    let raw = serde_json::to_string_pretty(&config.clone().normalized())
        .map_err(|error| format!("failed to serialize config: {error}"))?;
    fs::write(&path, raw).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn app_config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("exaterm").join("config.json"))
}

fn load_legacy_launcher_config() -> Option<AppConfig> {
    let raw = fs::read_to_string(legacy_launcher_config_path()?).ok()?;
    let legacy = serde_json::from_str::<LegacyLauncherConfig>(&raw).ok()?;
    Some(AppConfig {
        hosts: legacy.remotes,
        terminal: TerminalConfig::default(),
    })
}

fn legacy_launcher_config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("exaterm").join("launcher.json"))
}

#[derive(Deserialize)]
struct LegacyLauncherConfig {
    #[serde(default)]
    remotes: Vec<RememberedHost>,
}

#[cfg(test)]
mod tests {
    use super::{
        app_config_path, load_app_config, save_app_config, AppConfig, RememberedHost,
        TerminalConfig,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("exaterm-config-{label}-{nanos}"))
    }

    #[test]
    fn default_config_uses_terminal_defaults() {
        let config = AppConfig::default();
        assert!(!config.terminal.audible_bell);
    }

    #[test]
    fn remove_host_only_removes_matching_target() {
        let mut config = AppConfig {
            hosts: vec![
                RememberedHost {
                    target: "host-a".into(),
                    last_used_secs: 2,
                },
                RememberedHost {
                    target: "host-b".into(),
                    last_used_secs: 1,
                },
            ],
            terminal: TerminalConfig::default(),
        };

        assert!(config.remove_host(" host-a "));
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].target, "host-b");
        assert!(!config.remove_host("missing"));
    }

    #[test]
    fn saves_and_loads_config_from_xdg_config_home() {
        let _guard = crate::pet::env_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = unique_temp_dir("roundtrip");
        let old_config = std::env::var_os("XDG_CONFIG_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::env::remove_var("HOME");

        let config = AppConfig {
            hosts: vec![RememberedHost {
                target: "devbox".into(),
                last_used_secs: 42,
            }],
            terminal: TerminalConfig { audible_bell: true },
        };
        save_app_config(&config).expect("save config");
        assert_eq!(load_app_config(), config);
        assert_eq!(
            app_config_path().expect("config path"),
            dir.join("exaterm/config.json")
        );

        match old_config {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_legacy_launcher_hosts_when_new_config_is_absent() {
        let _guard = crate::pet::env_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let config_dir = unique_temp_dir("new-config");
        let state_dir = unique_temp_dir("legacy-state");
        let old_config = std::env::var_os("XDG_CONFIG_HOME");
        let old_state = std::env::var_os("XDG_STATE_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("XDG_CONFIG_HOME", &config_dir);
        std::env::set_var("XDG_STATE_HOME", &state_dir);
        std::env::remove_var("HOME");
        let legacy_path = state_dir.join("exaterm/launcher.json");
        fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("mkdir");
        fs::write(
            &legacy_path,
            r#"{"remotes":[{"target":"host-a","last_used_secs":7}]}"#,
        )
        .expect("write legacy config");

        let config = load_app_config();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].target, "host-a");
        assert!(config_dir.join("exaterm/config.json").exists());

        match old_config {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match old_state {
            Some(value) => std::env::set_var("XDG_STATE_HOME", value),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(config_dir);
        let _ = fs::remove_dir_all(state_dir);
    }
}
