use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_TERMINAL_ASSIST_MODEL: &str = "gpt-5.5-nano";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default, alias = "remotes")]
    pub hosts: Vec<RememberedHost>,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub terminal_assist: TerminalAssistConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hosts: Vec::new(),
            terminal: TerminalConfig::default(),
            terminal_assist: TerminalAssistConfig::default(),
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
        self.terminal_assist = self.terminal_assist.normalized();
        self
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAssistConfig {
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default = "default_openai_base_url_string")]
    pub openai_base_url: String,
    #[serde(default = "default_terminal_assist_model_string")]
    pub model: String,
}

impl Default for TerminalAssistConfig {
    fn default() -> Self {
        Self {
            openai_api_key: String::new(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.into(),
            model: DEFAULT_TERMINAL_ASSIST_MODEL.into(),
        }
    }
}

impl TerminalAssistConfig {
    pub fn normalized(mut self) -> Self {
        self.openai_api_key = self.openai_api_key.trim().to_string();
        self.openai_base_url =
            normalize_nonempty_or_default(&self.openai_base_url, DEFAULT_OPENAI_BASE_URL);
        self.model = normalize_nonempty_or_default(&self.model, DEFAULT_TERMINAL_ASSIST_MODEL);
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

pub fn apply_app_config_environment(config: &AppConfig) {
    apply_terminal_assist_config_environment(&config.terminal_assist);
}

pub fn apply_terminal_assist_config_environment(config: &TerminalAssistConfig) {
    let config = config.clone().normalized();
    if config.openai_api_key.is_empty() {
        env::remove_var("OPENAI_API_KEY");
    } else {
        env::set_var("OPENAI_API_KEY", &config.openai_api_key);
    }
    env::set_var("EXATERM_OPENAI_BASE_URL", &config.openai_base_url);
    env::set_var("EXATERM_TERMINAL_ASSIST_MODEL", &config.model);
}

fn load_legacy_launcher_config() -> Option<AppConfig> {
    let raw = fs::read_to_string(legacy_launcher_config_path()?).ok()?;
    let legacy = serde_json::from_str::<LegacyLauncherConfig>(&raw).ok()?;
    Some(AppConfig {
        hosts: legacy.remotes,
        terminal: TerminalConfig::default(),
        terminal_assist: TerminalAssistConfig::default(),
    })
}

fn legacy_launcher_config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("exaterm").join("launcher.json"))
}

fn normalize_nonempty_or_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.into()
    } else {
        value.into()
    }
}

fn default_openai_base_url_string() -> String {
    DEFAULT_OPENAI_BASE_URL.into()
}

fn default_terminal_assist_model_string() -> String {
    DEFAULT_TERMINAL_ASSIST_MODEL.into()
}

#[derive(Deserialize)]
struct LegacyLauncherConfig {
    #[serde(default)]
    remotes: Vec<RememberedHost>,
}

#[cfg(test)]
mod tests {
    use super::{
        app_config_path, apply_app_config_environment, load_app_config, save_app_config, AppConfig,
        RememberedHost, TerminalAssistConfig, TerminalConfig, DEFAULT_OPENAI_BASE_URL,
        DEFAULT_TERMINAL_ASSIST_MODEL,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("exaterm-config-{label}-{nanos}"))
    }

    #[test]
    fn default_config_uses_openai_terminal_assist_defaults() {
        let config = AppConfig::default();
        assert_eq!(
            config.terminal_assist.openai_base_url,
            DEFAULT_OPENAI_BASE_URL
        );
        assert_eq!(config.terminal_assist.model, DEFAULT_TERMINAL_ASSIST_MODEL);
        assert!(config.terminal_assist.openai_api_key.is_empty());
        assert!(!config.terminal.audible_bell);
    }

    #[test]
    fn saves_and_loads_config_from_xdg_config_home() {
        let _guard = ENV_MUTEX
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
            terminal_assist: TerminalAssistConfig {
                openai_api_key: "sk-test".into(),
                openai_base_url: "https://api.example.test/v1".into(),
                model: "gpt-5.5-nano".into(),
            },
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
        let _guard = ENV_MUTEX
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

    #[test]
    fn applies_terminal_assist_config_to_process_environment() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let old_key = std::env::var_os("OPENAI_API_KEY");
        let old_base = std::env::var_os("EXATERM_OPENAI_BASE_URL");
        let old_model = std::env::var_os("EXATERM_TERMINAL_ASSIST_MODEL");

        apply_app_config_environment(&AppConfig {
            hosts: Vec::new(),
            terminal: TerminalConfig::default(),
            terminal_assist: TerminalAssistConfig {
                openai_api_key: "sk-test".into(),
                openai_base_url: "https://api.example.test/v1".into(),
                model: "gpt-5.5-nano".into(),
            },
        });
        assert_eq!(
            std::env::var("OPENAI_API_KEY").ok().as_deref(),
            Some("sk-test")
        );
        assert_eq!(
            std::env::var("EXATERM_OPENAI_BASE_URL").ok().as_deref(),
            Some("https://api.example.test/v1")
        );
        assert_eq!(
            std::env::var("EXATERM_TERMINAL_ASSIST_MODEL")
                .ok()
                .as_deref(),
            Some("gpt-5.5-nano")
        );

        match old_key {
            Some(value) => std::env::set_var("OPENAI_API_KEY", value),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
        match old_base {
            Some(value) => std::env::set_var("EXATERM_OPENAI_BASE_URL", value),
            None => std::env::remove_var("EXATERM_OPENAI_BASE_URL"),
        }
        match old_model {
            Some(value) => std::env::set_var("EXATERM_TERMINAL_ASSIST_MODEL", value),
            None => std::env::remove_var("EXATERM_TERMINAL_ASSIST_MODEL"),
        }
    }
}
