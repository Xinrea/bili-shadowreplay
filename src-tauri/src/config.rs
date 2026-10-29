use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{self, AtomicU64};
use std::sync::Arc;

use crate::{danmu2ass::Danmu2AssOptions, recorder_manager::ClipRangeParams};

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub cache: String,
    pub output: String,
    pub live_start_notify: bool,
    pub live_end_notify: bool,
    pub clip_notify: bool,
    pub post_notify: bool,
    #[serde(default = "default_auto_subtitle")]
    pub auto_subtitle: bool,
    #[serde(default = "default_subtitle_generator_type")]
    pub subtitle_generator_type: String,
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    #[serde(default = "default_whisper_prompt")]
    pub whisper_prompt: String,
    /// Words PowerLive should favor when recognizing, shared by every room.
    #[serde(default)]
    pub powerlive_hotwords: Vec<String>,
    /// Room-specific words, keyed by platform then room ID.
    #[serde(default)]
    pub powerlive_room_hotwords: HashMap<String, HashMap<String, Vec<String>>>,
    /// The most characters a PowerLive subtitle line may have.
    #[serde(default = "default_powerlive_sentence_max_length")]
    pub powerlive_sentence_max_length: u32,
    #[serde(default = "default_openai_api_endpoint")]
    pub openai_api_endpoint: String,
    #[serde(default = "default_openai_api_key")]
    pub openai_api_key: String,
    #[serde(default = "default_clip_name_format")]
    pub clip_name_format: String,
    #[serde(default = "default_auto_generate_config")]
    pub auto_generate: AutoGenerateConfig,
    #[serde(default = "default_status_check_interval")]
    pub status_check_interval: u64,
    #[serde(skip)]
    pub config_path: String,
    #[serde(default = "default_whisper_language")]
    pub whisper_language: String,
    #[serde(default = "default_webhook_url")]
    pub webhook_url: String,
    #[serde(default = "default_danmu_ass_options")]
    pub danmu_ass_options: Danmu2AssOptions,
    #[serde(skip)]
    pub update_interval: Arc<AtomicU64>,
    #[serde(default = "default_llm_config")]
    pub llm: LlmConfig,
    #[serde(default)]
    pub mcp_enabled: bool,
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
}

pub const DEFAULT_MCP_PORT: u16 = 18763;

fn default_mcp_port() -> u16 {
    DEFAULT_MCP_PORT
}

fn update_mounted_config(path: &Path, content: &[u8]) -> io::Result<()> {
    // Linux cannot rename over a single-file bind mount. Update that inode
    // without truncating it first, preserving its permissions and mount.
    // Attempt to restore the original bytes on an I/O error; this fallback
    // cannot provide atomicity against a process crash or a failed rollback.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let mut previous = Vec::new();
    file.read_to_end(&mut previous)?;
    let write_contents = |file: &mut std::fs::File, bytes: &[u8]| -> io::Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes)?;
        file.set_len(bytes.len() as u64)?;
        file.sync_all()
    };
    if let Err(error) = write_contents(&mut file, content) {
        if let Err(rollback) = write_contents(&mut file, &previous) {
            return Err(io::Error::new(
                error.kind(),
                format!("{error}; restoring the previous configuration also failed: {rollback}"),
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LlmConfig {
    pub provider: String,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AutoGenerateConfig {
    pub enabled: bool,
    pub encode_danmu: bool,
    #[serde(default)]
    pub render_settings: crate::project::RenderSettings,
}

fn default_danmu_ass_options() -> Danmu2AssOptions {
    Danmu2AssOptions::default()
}

fn default_auto_subtitle() -> bool {
    false
}

fn default_subtitle_generator_type() -> String {
    "whisper".to_string()
}

fn default_whisper_model() -> String {
    "whisper_model.bin".to_string()
}

fn default_powerlive_sentence_max_length() -> u32 {
    crate::subtitle_generator::powerlive::DEFAULT_SENTENCE_MAX_LENGTH
}

fn default_whisper_prompt() -> String {
    "这是一段中文 你们好".to_string()
}

fn default_openai_api_endpoint() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_openai_api_key() -> String {
    String::new()
}

fn default_clip_name_format() -> String {
    "[{room_id}][{note}][{live_id}][{title}][{created_at}].mp4".to_string()
}

fn default_auto_generate_config() -> AutoGenerateConfig {
    AutoGenerateConfig {
        enabled: false,
        encode_danmu: false,
        render_settings: crate::project::RenderSettings::default(),
    }
}

fn default_status_check_interval() -> u64 {
    30
}

fn default_whisper_language() -> String {
    "auto".to_string()
}

fn default_webhook_url() -> String {
    String::new()
}

fn default_llm_config() -> LlmConfig {
    LlmConfig {
        provider: "openai".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        api_key: String::new(),
        model: String::new(),
    }
}

impl Config {
    /// Update one dictionary, returning its normalized words (not the merged list).
    /// Omitted scope selects global; a room scope requires both nonblank keys.
    pub fn set_powerlive_hotwords(
        &mut self,
        hotwords: Vec<String>,
        platform: Option<&str>,
        room_id: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let scope = match (platform, room_id) {
            (None, None) => None,
            (Some(platform), Some(room_id))
                if !platform.trim().is_empty() && !room_id.trim().is_empty() =>
            {
                Some((platform, room_id))
            }
            _ => return Err("platform and roomId must both be nonempty or both omitted".into()),
        };
        let hotwords = crate::subtitle_generator::powerlive::normalize_hotwords(hotwords);
        if let Some((platform, room_id)) = scope {
            if hotwords.is_empty() {
                if let Some(rooms) = self.powerlive_room_hotwords.get_mut(platform) {
                    rooms.remove(room_id);
                    if rooms.is_empty() {
                        self.powerlive_room_hotwords.remove(platform);
                    }
                }
            } else {
                self.powerlive_room_hotwords
                    .entry(platform.to_string())
                    .or_default()
                    .insert(room_id.to_string(), hotwords.clone());
            }
        } else {
            self.powerlive_hotwords = hotwords.clone();
        }
        Ok(hotwords)
    }

    pub fn load(
        config_path: &PathBuf,
        default_cache: &Path,
        default_output: &Path,
    ) -> Result<Self, String> {
        let config_path_str = config_path
            .to_str()
            .ok_or_else(|| format!("Config path is not valid UTF-8: {}", config_path.display()))?
            .to_string();
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(mut config) = toml::from_str::<Config>(&content) {
                config.config_path = config_path_str;
                config.update_interval = Arc::new(AtomicU64::new(config.status_check_interval));
                return Ok(config);
            }
        }

        if let Some(dir_path) = PathBuf::from(config_path).parent() {
            if let Err(e) = std::fs::create_dir_all(dir_path) {
                return Err(format!("Failed to create config dir: {e}"));
            }
        }

        let config = Config {
            cache: default_cache.to_string_lossy().into_owned(),
            output: default_output.to_string_lossy().into_owned(),
            live_start_notify: true,
            live_end_notify: true,
            clip_notify: true,
            post_notify: true,
            auto_subtitle: false,
            subtitle_generator_type: default_subtitle_generator_type(),
            whisper_model: default_whisper_model(),
            whisper_prompt: default_whisper_prompt(),
            powerlive_hotwords: Vec::new(),
            powerlive_room_hotwords: HashMap::new(),
            powerlive_sentence_max_length: default_powerlive_sentence_max_length(),
            openai_api_endpoint: default_openai_api_endpoint(),
            openai_api_key: default_openai_api_key(),
            clip_name_format: default_clip_name_format(),
            auto_generate: default_auto_generate_config(),
            status_check_interval: default_status_check_interval(),
            config_path: config_path_str,
            whisper_language: default_whisper_language(),
            webhook_url: default_webhook_url(),
            danmu_ass_options: default_danmu_ass_options(),
            update_interval: Arc::new(AtomicU64::new(default_status_check_interval())),
            llm: default_llm_config(),
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
        };

        config.save();

        Ok(config)
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            log::error!("{error}");
        }
    }

    pub fn try_save(&self) -> Result<(), String> {
        self.try_save_with(|pending, path| pending.persist(path))
    }

    fn try_save_with(
        &self,
        persist: impl FnOnce(
            tempfile::NamedTempFile,
            &Path,
        ) -> Result<std::fs::File, tempfile::PersistError>,
    ) -> Result<(), String> {
        let content = toml::to_string(self)
            .map_err(|error| format!("Failed to serialize config: {error}"))?;
        // Follow existing symlinks and replace the file only after a complete
        // write, so a failed settings change cannot truncate the old config.
        let path = std::fs::canonicalize(&self.config_path)
            .unwrap_or_else(|_| PathBuf::from(&self.config_path));
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let failure = |error| format!("Failed to save config {}: {error}", self.config_path);
        let mut pending = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        pending.write_all(content.as_bytes()).map_err(failure)?;
        if let Ok(metadata) = std::fs::metadata(&path) {
            pending
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(failure)?;
        }
        pending.as_file().sync_all().map_err(failure)?;
        match persist(pending, &path) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == io::ErrorKind::ResourceBusy => {
                drop(error.file);
                update_mounted_config(&path, content.as_bytes()).map_err(failure)
            }
            Err(error) => Err(failure(error.error)),
        }
    }

    #[allow(dead_code)]
    pub fn set_cache_path(&mut self, path: &str) {
        self.cache = path.to_string();
        self.save();
    }

    #[allow(dead_code)]
    pub fn set_output_path(&mut self, path: &str) {
        self.output = path.into();
        self.save();
    }

    pub fn set_whisper_language(&mut self, language: &str) {
        self.whisper_language = language.to_string();
        self.save();
    }

    #[allow(dead_code)]
    pub fn set_danmu_ass_options(&mut self, options: Danmu2AssOptions) {
        self.danmu_ass_options = options;
        self.save();
    }

    pub fn generate_clip_name(&self, params: &ClipRangeParams) -> PathBuf {
        // get format config
        // filter special characters from title to make sure file name is valid
        let title = params
            .title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let format_config = self.clip_name_format.clone();
        let format_config = format_config.replace("{title}", &title);
        let format_config = format_config.replace("{platform}", &params.platform);
        let format_config = format_config.replace("{room_id}", &params.room_id.to_string());
        let format_config = format_config.replace("{live_id}", &params.live_id);
        let format_config = format_config.replace("{note}", &params.note);
        let format_config = format_config.replace(
            "{x}",
            &params
                .ranges
                .first()
                .map_or("0".to_string(), |r| r.start.to_string()),
        );
        let format_config = format_config.replace(
            "{y}",
            &params
                .ranges
                .last()
                .map_or("0".to_string(), |r| r.end.to_string()),
        );
        let format_config = format_config.replace(
            "{created_at}",
            &Local::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
        );
        let duration = params.ranges.iter().map(|r| r.duration()).sum::<f64>();
        let format_config = format_config.replace("{length}", &duration.to_string());

        let mut format_config = format_config;
        while format_config.contains("[]") {
            format_config = format_config.replace("[]", "");
        }

        let sanitized = sanitize_filename::sanitize(&format_config);
        let output = self.output.clone();

        Path::new(&output).join(&sanitized)
    }

    pub fn set_status_check_interval(&mut self, interval: u64) {
        self.status_check_interval = interval;
        self.update_interval
            .store(interval, atomic::Ordering::Relaxed);
        self.save();
    }
}

#[cfg(test)]
mod powerlive_config_tests {
    use super::*;

    fn config() -> Config {
        toml::from_str(include_str!("../config.example.toml")).unwrap()
    }

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn powerlive_dictionaries_default_for_legacy_and_new_configs() {
        let legacy = config();
        assert!(legacy.powerlive_hotwords.is_empty());
        assert!(legacy.powerlive_room_hotwords.is_empty());
        let mut json = serde_json::to_value(&legacy).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("powerlive_room_hotwords");
        json["powerlive_hotwords"] = serde_json::json!(["legacy"]);
        let legacy: Config = serde_json::from_value(json).unwrap();
        assert_eq!(legacy.powerlive_hotwords, ["legacy"]);
        assert!(legacy.powerlive_room_hotwords.is_empty());

        let directory = tempfile::tempdir().unwrap();
        let fresh = Config::load(
            &directory.path().join("config.toml"),
            directory.path(),
            directory.path(),
        )
        .unwrap();
        assert!(fresh.powerlive_room_hotwords.is_empty());
    }

    #[test]
    fn powerlive_updates_are_normalized_isolated_and_persisted() {
        let mut config = config();
        assert_eq!(
            config
                .set_powerlive_hotwords(words(&[" global ", "global", ""]), None, None)
                .unwrap(),
            ["global"]
        );
        let set_room = |config: &mut Config, platform, room, values: &[&str]| {
            config
                .set_powerlive_hotwords(words(values), Some(platform), Some(room))
                .unwrap()
        };
        assert_eq!(
            set_room(
                &mut config,
                "bilibili",
                "123",
                &[" Room   word ", "Room word"]
            ),
            ["Room word"]
        );
        set_room(&mut config, "bilibili", "456", &["other room"]);
        set_room(&mut config, "douyu", "123", &["other platform"]);
        assert_eq!(config.powerlive_hotwords, ["global"]);
        assert_eq!(
            config.powerlive_room_hotwords["bilibili"]["123"],
            ["Room word"]
        );
        assert_eq!(
            config.powerlive_room_hotwords["bilibili"]["456"],
            ["other room"]
        );
        assert_eq!(
            config.powerlive_room_hotwords["douyu"]["123"],
            ["other platform"]
        );

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        config.config_path = path.to_string_lossy().into_owned();
        config.try_save().unwrap();
        let reloaded = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert_eq!(reloaded.powerlive_hotwords, config.powerlive_hotwords);
        assert_eq!(
            reloaded.powerlive_room_hotwords,
            config.powerlive_room_hotwords
        );

        // Normalized-empty words remove only the selected room and prune empty platforms.
        set_room(&mut config, "bilibili", "123", &[" ", "\t"]);
        assert!(!config.powerlive_room_hotwords["bilibili"].contains_key("123"));
        set_room(&mut config, "bilibili", "456", &[]);
        assert!(!config.powerlive_room_hotwords.contains_key("bilibili"));
        set_room(&mut config, "missing", "123", &[]);
        assert!(!config.powerlive_room_hotwords.contains_key("missing"));
        assert_eq!(
            config.powerlive_room_hotwords["douyu"]["123"],
            ["other platform"]
        );
        config
            .set_powerlive_hotwords(Vec::new(), None, None)
            .unwrap();
        assert!(config.powerlive_hotwords.is_empty());
        assert_eq!(config.powerlive_room_hotwords.len(), 1);
    }

    #[test]
    fn powerlive_invalid_scopes_leave_both_dictionaries_unchanged() {
        let mut config = config();
        config
            .set_powerlive_hotwords(words(&["global"]), None, None)
            .unwrap();
        config
            .set_powerlive_hotwords(words(&["room"]), Some("bilibili"), Some("123"))
            .unwrap();
        let before = serde_json::to_value(&config).unwrap();
        for (platform, room_id) in [
            (Some("bilibili"), None),
            (None, Some("123")),
            (Some(""), Some("123")),
            (Some("bilibili"), Some("")),
            (Some(" \t"), Some("123")),
            (Some("bilibili"), Some(" \n")),
            (Some(""), Some("")),
        ] {
            assert!(config
                .set_powerlive_hotwords(words(&["bad"]), platform, room_id)
                .is_err());
            assert_eq!(serde_json::to_value(&config).unwrap(), before);
        }
    }
}

#[cfg(test)]
mod auto_generate_tests {
    use super::*;
    use crate::project::{EncodingPriority, RenderSettings, VideoCodec};

    #[test]
    fn auto_generate_defaults_migrate_and_persist_encoding_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let fresh = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert_eq!(
            fresh.auto_generate.render_settings,
            RenderSettings::default()
        );
        assert!(!fresh.auto_generate.enabled);
        let mut legacy = toml::Value::try_from(&fresh).unwrap();
        legacy["auto_generate"]
            .as_table_mut()
            .unwrap()
            .remove("render_settings");
        std::fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
        let mut loaded = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert_eq!(
            loaded.auto_generate.render_settings,
            RenderSettings::default()
        );
        loaded.auto_generate.enabled = true;
        loaded.auto_generate.encode_danmu = true;
        loaded.auto_generate.render_settings = RenderSettings {
            codec: VideoCodec::H264,
            priority: EncodingPriority::Quality,
            bitrate_kbps: 16_000,
            encoding: Default::default(),
        };
        loaded.try_save().unwrap();
        let reloaded = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert_eq!(reloaded.auto_generate, loaded.auto_generate);
        legacy.as_table_mut().unwrap().remove("auto_generate");
        let missing: Config = legacy.try_into().unwrap();
        assert_eq!(missing.auto_generate, fresh.auto_generate);
    }
}

#[cfg(test)]
mod mcp_config_tests {
    use super::*;

    #[test]
    fn mcp_is_disabled_for_existing_and_new_configs() {
        let old = include_str!("../config.example.toml")
            .replace("mcp_enabled = false\n", "")
            .replace("mcp_port = 18763\n", "");
        let old = toml::from_str::<Config>(&old).unwrap();
        assert!(!old.mcp_enabled);
        assert_eq!(old.mcp_port, DEFAULT_MCP_PORT);
        let directory = tempfile::tempdir().unwrap();
        let config = Config::load(
            &directory.path().join("config.toml"),
            directory.path(),
            directory.path(),
        )
        .unwrap();
        assert!(!config.mcp_enabled);
        assert_eq!(config.mcp_port, DEFAULT_MCP_PORT);
    }

    #[test]
    fn mcp_setting_persists_and_save_failures_are_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut config = Config::load(&path, directory.path(), directory.path()).unwrap();
        config.mcp_enabled = true;
        config.mcp_port = 18764;
        config.try_save().unwrap();
        let reloaded = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert!(reloaded.mcp_enabled);
        assert_eq!(reloaded.mcp_port, 18764);
        config.config_path = directory.path().to_string_lossy().into_owned();
        assert!(config.try_save().is_err());
    }

    #[test]
    fn busy_replace_updates_existing_inode_and_removes_trailing_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut config = Config::load(&path, directory.path(), directory.path()).unwrap();
        std::fs::OpenOptions::new().append(true).open(&path).unwrap()
            .write_all(b"\n# Long trailing comment which must disappear when replacing the shorter configuration\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let previous_metadata = std::fs::metadata(&path).unwrap();
        config.mcp_enabled = true;
        config.mcp_port = 19763;
        let expected = toml::to_string(&config).unwrap();
        assert!((expected.len() as u64) < previous_metadata.len());
        config
            .try_save_with(|pending, _| {
                Err(tempfile::PersistError {
                    error: io::Error::from(io::ErrorKind::ResourceBusy),
                    file: pending,
                })
            })
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        let saved = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert!(saved.mcp_enabled);
        assert_eq!(saved.mcp_port, 19763);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let saved_metadata = std::fs::metadata(&path).unwrap();
            assert_eq!(saved_metadata.ino(), previous_metadata.ino());
            assert_eq!(saved_metadata.dev(), previous_metadata.dev());
            assert_eq!(
                saved_metadata.permissions().mode(),
                previous_metadata.permissions().mode()
            );
        }
    }

    #[test]
    fn non_busy_replace_errors_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut config = Config::load(&path, directory.path(), directory.path()).unwrap();
        let original = std::fs::read(&path).unwrap();
        config.mcp_enabled = true;
        config.mcp_port = 19763;
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert!(config
                .try_save_with(|pending, _| Err(tempfile::PersistError {
                    error: io::Error::from(kind),
                    file: pending,
                }))
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        let saved = Config::load(&path, directory.path(), directory.path()).unwrap();
        assert!(!saved.mcp_enabled);
        assert_eq!(saved.mcp_port, DEFAULT_MCP_PORT);
    }
}
