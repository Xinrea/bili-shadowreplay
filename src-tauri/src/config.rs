use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{recorder::PlatformType, recorder_manager::ClipRangeParams};

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
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    #[serde(default = "default_whisper_prompt")]
    pub whisper_prompt: String,
    #[serde(default = "default_clip_name_format")]
    pub clip_name_format: String,
    #[serde(default = "default_auto_generate_config")]
    pub auto_generate: AutoGenerateConfig,
    #[serde(skip)]
    pub config_path: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AutoGenerateConfig {
    pub enabled: bool,
    pub encode_danmu: bool,
}

fn default_auto_subtitle() -> bool {
    false
}

fn default_whisper_model() -> String {
    "".to_string()
}

fn default_whisper_prompt() -> String {
    "这是一段中文 你们好".to_string()
}

fn default_clip_name_format() -> String {
    "[{room_id}][{live_id}][{title}][{created_at}].mp4".to_string()
}

fn default_auto_generate_config() -> AutoGenerateConfig {
    AutoGenerateConfig {
        enabled: false,
        encode_danmu: false,
    }
}

impl Config {
    pub fn load(config_path: &str) -> Self {
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        }
        let config = Config {
            cache: "./cache".to_string(),
            output: "./output".to_string(),
            live_start_notify: true,
            live_end_notify: true,
            clip_notify: true,
            post_notify: true,
            auto_subtitle: false,
            whisper_model: "".to_string(),
            whisper_prompt: "这是一段中文 你们好".to_string(),
            clip_name_format: "[{room_id}][{live_id}][{title}][{created_at}].mp4".to_string(),
            auto_generate: default_auto_generate_config(),
            config_path: config_path.to_string(),
        };
        config.save();
        config
    }

    pub fn save(&self) {
        let content = toml::to_string(&self).unwrap();
        std::fs::write(self.config_path.clone(), content).unwrap();
    }

    pub fn set_cache_path(&mut self, path: &str) {
        self.cache = path.to_string();
        self.save();
    }

    pub fn set_output_path(&mut self, path: &str) {
        self.output = path.into();
        self.save();
    }

    pub fn generate_clip_name(&self, params: &ClipRangeParams) -> PathBuf {
        let platform = PlatformType::from_str(&params.platform).unwrap();

        // get format config
        // filter special characters from title to make sure file name is valid
        let title = params
            .title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let format_config = self.clip_name_format.clone();
        let format_config = format_config.replace("{title}", &title);
        let format_config = format_config.replace("{platform}", platform.as_str());
        let format_config = format_config.replace("{room_id}", &params.room_id.to_string());
        let format_config = format_config.replace("{live_id}", &params.live_id);
        let format_config = format_config.replace("{x}", &params.x.to_string());
        let format_config = format_config.replace("{y}", &params.y.to_string());
        let format_config = format_config.replace(
            "{created_at}",
            &Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
        );
        let format_config = format_config.replace("{length}", &(params.y - params.x).to_string());

        let output = self.output.clone();

        Path::new(&output).join(&format_config)
    }
}
