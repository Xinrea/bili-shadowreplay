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
    #[serde(default = "default_subtitle_generator_type")]
    pub subtitle_generator_type: String,
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    #[serde(default = "default_whisper_prompt")]
    pub whisper_prompt: String,
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
    pub whisper_language: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AutoGenerateConfig {
    pub enabled: bool,
    pub encode_danmu: bool,
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

fn default_whisper_prompt() -> String {
    "这是一段中文 你们好".to_string()
}

fn default_openai_api_endpoint() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_openai_api_key() -> String {
    "".to_string()
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

fn default_status_check_interval() -> u64 {
    30
}

fn default_whisper_language() -> Option<String> {
    None
}

impl Config {
    pub fn load(
        config_path: &PathBuf,
        default_cache: &Path,
        default_output: &Path,
    ) -> Result<Self, String> {
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(mut config) = toml::from_str::<Config>(&content) {
                config.config_path = config_path.to_str().unwrap().into();
                return Ok(config);
            }
        }

        if let Some(dir_path) = PathBuf::from(config_path).parent() {
            if let Err(e) = std::fs::create_dir_all(dir_path) {
                return Err(format!("Failed to create config dir: {e}"));
            }
        }

        let config = Config {
            cache: default_cache.to_str().unwrap().into(),
            output: default_output.to_str().unwrap().into(),
            live_start_notify: true,
            live_end_notify: true,
            clip_notify: true,
            post_notify: true,
            auto_subtitle: false,
            subtitle_generator_type: default_subtitle_generator_type(),
            whisper_model: default_whisper_model(),
            whisper_prompt: default_whisper_prompt(),
            openai_api_endpoint: default_openai_api_endpoint(),
            openai_api_key: default_openai_api_key(),
            clip_name_format: default_clip_name_format(),
            auto_generate: default_auto_generate_config(),
            status_check_interval: default_status_check_interval(),
            config_path: config_path.to_str().unwrap().into(),
            whisper_language: None,
        };

        config.save();

        Ok(config)
    }

    pub fn save(&self) {
        let content = toml::to_string(&self).unwrap();
        if let Err(e) = std::fs::write(self.config_path.clone(), content) {
            log::error!("Failed to save config: {} {}", e, self.config_path);
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

    #[allow(dead_code)]
    pub fn set_whisper_language(&mut self, language: Option<&str>) {
        self.whisper_language = language.map(|s| s.to_string());
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
