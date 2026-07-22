use exaterm_types::model::PetProfile;
use exaterm_types::proto::PetOrigin;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CLIENT_ORIGIN_FILE: &str = "pet-origin.json";
const PET_PROFILE_FILE: &str = "pet-profile.json";
const MAX_NAME_CHARS: usize = 40;
const MAX_TRAIT_CHARS: usize = 180;
const MAX_BACKSTORY_CHARS: usize = 900;
const MAX_APPEARANCE_LINES: usize = 8;
const MAX_APPEARANCE_COLUMNS: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PetProfileDraft {
    pub name: String,
    pub appearance_ascii: String,
    pub temperament: String,
    pub backstory: String,
    pub comment_style: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredClientOrigin {
    install_id: String,
}

pub fn load_pet_profile() -> Option<PetProfile> {
    let path = pet_profile_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let profile = serde_json::from_str::<PetProfile>(&raw).ok()?;
    validate_pet_profile(profile).ok()
}

pub fn save_pet_profile(profile: &PetProfile) -> Result<(), String> {
    let path = pet_profile_path().ok_or("could not determine Exaterm state path")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create pet profile directory: {error}"))?;
    }
    let raw = serde_json::to_string_pretty(profile)
        .map_err(|error| format!("failed to serialize pet profile: {error}"))?;
    fs::write(&path, raw).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn pet_profile_path() -> Option<PathBuf> {
    exaterm_state_dir().map(|dir| dir.join(PET_PROFILE_FILE))
}

pub fn load_or_create_client_pet_origin() -> Result<PetOrigin, String> {
    let install_id = load_or_create_client_install_id()?;
    Ok(PetOrigin {
        seed_hash: pet_seed_hash(client_origin_material(&install_id)),
    })
}

pub fn host_pet_origin() -> PetOrigin {
    PetOrigin {
        seed_hash: pet_seed_hash(host_origin_material()),
    }
}

pub fn pet_seed_hash(material: impl AsRef<[u8]>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in material.as_ref() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn profile_from_draft(draft: PetProfileDraft, seed_hash: &str) -> Result<PetProfile, String> {
    validate_seed_hash(seed_hash)?;
    validate_pet_profile(PetProfile {
        name: sanitize_single_line(&draft.name, MAX_NAME_CHARS, "name")?,
        appearance_ascii: sanitize_ascii_art(&draft.appearance_ascii)?,
        temperament: sanitize_single_line(&draft.temperament, MAX_TRAIT_CHARS, "temperament")?,
        backstory: sanitize_multiline_text(&draft.backstory, MAX_BACKSTORY_CHARS, "backstory")?,
        comment_style: sanitize_single_line(
            &draft.comment_style,
            MAX_TRAIT_CHARS,
            "comment_style",
        )?,
        seed_hash: seed_hash.to_string(),
    })
}

pub fn bootstrap_pet_profile(seed_hash: &str) -> PetProfile {
    let variants = [
        (
            "Bracket",
            "[._.]\n /|\\",
            "dry and watchful",
            "Turned up between two terminal redraws and stayed for the logs.",
            "short, technical, mildly sarcastic",
        ),
        (
            "Tilde",
            "~[o_o]~\n  /\\",
            "quiet and exacting",
            "Lives near the prompt and distrusts unexplained success.",
            "terse, observant, operational",
        ),
        (
            "Cursor",
            "<._.>_\n  ||",
            "patient and skeptical",
            "Waited at the end of a command until the workspace noticed.",
            "brief, dry, evidence-first",
        ),
    ];
    let index =
        (u64::from_str_radix(seed_hash, 16).unwrap_or_default() % variants.len() as u64) as usize;
    let (name, appearance_ascii, temperament, backstory, comment_style) = variants[index];
    PetProfile {
        name: name.into(),
        appearance_ascii: appearance_ascii.into(),
        temperament: temperament.into(),
        backstory: backstory.into(),
        comment_style: comment_style.into(),
        seed_hash: seed_hash.into(),
    }
}

pub fn validate_pet_profile(profile: PetProfile) -> Result<PetProfile, String> {
    validate_seed_hash(&profile.seed_hash)?;
    Ok(PetProfile {
        name: sanitize_single_line(&profile.name, MAX_NAME_CHARS, "name")?,
        appearance_ascii: sanitize_ascii_art(&profile.appearance_ascii)?,
        temperament: sanitize_single_line(&profile.temperament, MAX_TRAIT_CHARS, "temperament")?,
        backstory: sanitize_multiline_text(&profile.backstory, MAX_BACKSTORY_CHARS, "backstory")?,
        comment_style: sanitize_single_line(
            &profile.comment_style,
            MAX_TRAIT_CHARS,
            "comment_style",
        )?,
        seed_hash: profile.seed_hash,
    })
}

pub fn sanitize_pet_comment_message(message: &str) -> Result<String, String> {
    sanitize_single_line(message, 180, "message")
}

pub fn clamp_pet_comment_ttl(ttl_secs: Option<u64>) -> u64 {
    ttl_secs.unwrap_or(8).clamp(3, 30)
}

fn validate_seed_hash(seed_hash: &str) -> Result<(), String> {
    let valid = seed_hash.len() == 16 && seed_hash.chars().all(|ch| ch.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err("seed_hash must be 16 hex characters".into())
    }
}

fn sanitize_single_line(value: &str, max_chars: usize, field: &str) -> Result<String, String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if !trimmed.chars().all(|ch| ch.is_ascii_graphic() || ch == ' ') {
        return Err(format!("{field} must be printable ASCII"));
    }
    Ok(trimmed.chars().take(max_chars).collect())
}

fn sanitize_multiline_text(value: &str, max_chars: usize, field: &str) -> Result<String, String> {
    let normalized = value.replace('\r', "\n");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if !trimmed
        .chars()
        .all(|ch| ch == '\n' || ch.is_ascii_graphic() || ch == ' ')
    {
        return Err(format!("{field} must be printable ASCII"));
    }
    Ok(trimmed.chars().take(max_chars).collect())
}

fn sanitize_ascii_art(value: &str) -> Result<String, String> {
    let normalized = value.replace('\r', "\n");
    let trimmed = normalized.trim_matches('\n');
    if trimmed.trim().is_empty() {
        return Err("appearance_ascii cannot be empty".into());
    }
    if !trimmed
        .chars()
        .all(|ch| ch == '\n' || ch.is_ascii_graphic() || ch == ' ')
    {
        return Err("appearance_ascii must be printable ASCII".into());
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() > MAX_APPEARANCE_LINES {
        return Err(format!(
            "appearance_ascii must be at most {MAX_APPEARANCE_LINES} lines"
        ));
    }
    if lines
        .iter()
        .any(|line| line.chars().count() > MAX_APPEARANCE_COLUMNS)
    {
        return Err(format!(
            "appearance_ascii lines must be at most {MAX_APPEARANCE_COLUMNS} columns"
        ));
    }
    Ok(trimmed.to_string())
}

fn load_or_create_client_install_id() -> Result<String, String> {
    let path = exaterm_state_dir()
        .ok_or("could not determine Exaterm state path")?
        .join(CLIENT_ORIGIN_FILE);
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(stored) = serde_json::from_str::<StoredClientOrigin>(&raw) {
            if !stored.install_id.trim().is_empty() {
                return Ok(stored.install_id);
            }
        }
    }

    let install_id = new_install_id();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create pet origin directory: {error}"))?;
    }
    let raw = serde_json::to_string_pretty(&StoredClientOrigin {
        install_id: install_id.clone(),
    })
    .map_err(|error| format!("failed to serialize pet origin: {error}"))?;
    fs::write(&path, raw)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    Ok(install_id)
}

fn new_install_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    pet_seed_hash(format!(
        "install:{}:{}:{}",
        std::process::id(),
        nanos,
        host_origin_material()
    ))
}

fn client_origin_material(install_id: &str) -> String {
    format!(
        "exaterm-pet-client:v1\ninstall={install_id}\nhost={}",
        host_origin_material()
    )
}

fn host_origin_material() -> String {
    let machine_id =
        read_first_existing(&["/etc/machine-id", "/var/lib/dbus/machine-id"]).unwrap_or_default();
    let hostname = read_first_existing(&["/proc/sys/kernel/hostname"])
        .or_else(|| env::var("HOSTNAME").ok())
        .unwrap_or_default();
    format!(
        "os={}\narch={}\nmachine={}\nhost={}",
        env::consts::OS,
        env::consts::ARCH,
        machine_id.trim(),
        hostname.trim()
    )
}

fn read_first_existing(paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| fs::read_to_string(path).ok())
}

fn exaterm_state_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("exaterm"))
}

#[cfg(test)]
pub(crate) fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_state(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        env::temp_dir().join(format!("exaterm-pet-{label}-{nanos}"))
    }

    fn draft() -> PetProfileDraft {
        PetProfileDraft {
            name: "Bracket".into(),
            appearance_ascii: "/\\_/\\\\\n( -.- )".into(),
            temperament: "dry and watchful".into(),
            backstory: "Spawned from a terminal resize that nobody can reproduce.".into(),
            comment_style: "short, technical, mildly sarcastic".into(),
        }
    }

    #[test]
    fn profile_from_draft_sanitizes_and_attaches_seed() {
        let profile = profile_from_draft(draft(), "0123456789abcdef").expect("profile");
        assert_eq!(profile.name, "Bracket");
        assert_eq!(profile.seed_hash, "0123456789abcdef");
        assert!(profile.appearance_ascii.contains("( -.- )"));
    }

    #[test]
    fn bootstrap_profile_is_valid_and_stable_for_seed() {
        let first = bootstrap_pet_profile("0123456789abcdef");
        let second = bootstrap_pet_profile("0123456789abcdef");

        assert_eq!(first, second);
        assert_eq!(validate_pet_profile(first.clone()).unwrap(), first);
    }

    #[test]
    fn profile_validation_rejects_controls_and_oversized_art() {
        let mut bad = draft();
        bad.appearance_ascii = "\x1b[31mnope".into();
        assert!(profile_from_draft(bad, "0123456789abcdef").is_err());

        let mut wide = draft();
        wide.appearance_ascii = "x".repeat(49);
        assert!(profile_from_draft(wide, "0123456789abcdef").is_err());
    }

    #[test]
    fn profile_storage_uses_xdg_state_home() {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = temp_state("storage");
        let old_state = env::var_os("XDG_STATE_HOME");
        let old_home = env::var_os("HOME");
        env::set_var("XDG_STATE_HOME", &dir);
        env::remove_var("HOME");

        let profile = profile_from_draft(draft(), "0123456789abcdef").expect("profile");
        save_pet_profile(&profile).expect("save");
        assert_eq!(load_pet_profile(), Some(profile));
        assert_eq!(
            pet_profile_path().expect("profile path"),
            dir.join("exaterm").join(PET_PROFILE_FILE)
        );

        match old_state {
            Some(value) => env::set_var("XDG_STATE_HOME", value),
            None => env::remove_var("XDG_STATE_HOME"),
        }
        match old_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn client_origin_is_stable_after_first_creation() {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = temp_state("origin");
        let old_state = env::var_os("XDG_STATE_HOME");
        let old_home = env::var_os("HOME");
        env::set_var("XDG_STATE_HOME", &dir);
        env::remove_var("HOME");

        let first = load_or_create_client_pet_origin().expect("origin");
        let second = load_or_create_client_pet_origin().expect("origin");
        assert_eq!(first, second);

        match old_state {
            Some(value) => env::set_var("XDG_STATE_HOME", value),
            None => env::remove_var("XDG_STATE_HOME"),
        }
        match old_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(dir);
    }
}
