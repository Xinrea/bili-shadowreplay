use platform_dirs::AppDirs;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub cache: String,
    pub output: String,
    pub primary_uid: u64,
    pub webid: String,
    pub webid_ts: i64,
    pub live_start_notify: bool,
    pub live_end_notify: bool,
    pub clip_notify: bool,
    pub post_notify: bool,
    #[serde(default = "default_auto_subtitle")]
    pub auto_subtitle: bool,
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    #[serde(default = "default_clip_name_format")]
    pub clip_name_format: String,
}

fn default_auto_subtitle() -> bool {
    false
}

fn default_whisper_model() -> String {
    "".to_string()
}

fn default_clip_name_format() -> String {
    "[{room_id}][{live_id}][{title}][{created_at}].mp4".to_string()
}

impl Config {
    pub fn load() -> Self {
        let app_dirs = AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false).unwrap();
        let config_path = app_dirs.config_dir.join("Conf.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        }
        let config = Config {
            webid: "".to_string(),
            webid_ts: 0,
            cache: app_dirs
                .cache_dir
                .join("cache")
                .to_str()
                .unwrap()
                .to_string(),
            output: app_dirs
                .data_dir
                .join("output")
                .to_str()
                .unwrap()
                .to_string(),
            primary_uid: 0,
            live_start_notify: true,
            live_end_notify: true,
            clip_notify: true,
            post_notify: true,
            auto_subtitle: false,
            whisper_model: "".to_string(),
            clip_name_format: "[{room_id}][{live_id}][{title}][{created_at}].mp4".to_string(),
        };
        config.save();
        config
    }

    pub fn save(&self) {
        let content = toml::to_string(&self).unwrap();
        let app_dirs = AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false).unwrap();
        // Create app dirs if not exists
        std::fs::create_dir_all(&app_dirs.config_dir).unwrap();
        let config_path = app_dirs.config_dir.join("Conf.toml");
        std::fs::write(config_path, content).unwrap();
    }

    pub fn set_cache_path(&mut self, path: &str) {
        self.cache = path.to_string();
        self.save();
    }

    pub fn set_output_path(&mut self, path: &str) {
        self.output = path.into();
        self.save();
    }

    pub fn webid_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        // expire in 20 hours
        now - self.webid_ts > 72000
    }
}
