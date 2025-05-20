// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive_migration;
mod config;
mod danmu2ass;
mod database;
mod ffmpeg;
mod handlers;
#[cfg(feature = "headless")]
mod http_server;
#[cfg(feature = "headless")]
mod migration;
mod progress_manager;
mod progress_reporter;
mod recorder;
mod recorder_manager;
mod state;
mod subtitle_generator;
#[cfg(feature = "gui")]
mod tray;

use archive_migration::try_rebuild_archives;
use async_std::fs;
use chrono::Utc;
use config::Config;
use database::Database;
use recorder::bilibili::client::BiliClient;
use recorder_manager::RecorderManager;
use simplelog::ConfigBuilder;
use state::State;
use std::path::Path;
use std::sync::Arc;
use std::{fs::File, os::unix::fs::MetadataExt};
use tokio::sync::RwLock;

#[cfg(feature = "gui")]
use {
    recorder::PlatformType,
    tauri::{Manager, WindowEvent},
    tauri_plugin_sql::{Migration, MigrationKind},
};

#[cfg(feature = "headless")]
use {
    clap::{arg, command, Parser},
    futures_core::future::BoxFuture,
    migration::{Migration, MigrationKind},
    sqlx::error::BoxDynError,
    sqlx::migrate::Migration as SqlxMigration,
    sqlx::migrate::MigrationSource,
    sqlx::{
        migrate::{MigrateDatabase, Migrator},
        Pool, Sqlite,
    },
};

/// open a log file, if file size exceeds 1MB, backup log file and create a new one.
async fn open_log_file(log_dir: &Path) -> Result<File, Box<dyn std::error::Error>> {
    let log_filename = log_dir.join("bsr.log");

    if let Ok(meta) = fs::metadata(&log_filename).await {
        if meta.size() > 1024 * 1024 {
            // move original file to backup
            let date_str = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
            let backup_filename = log_dir.join(&format!("bsr-{date_str}.log"));
            let _ = fs::rename(&log_filename, backup_filename).await?;
        }
    }

    Ok(File::options()
        .create(true)
        .append(true)
        .open(&log_filename)?)
}

async fn setup_logging(log_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // mkdir if not exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)?;
    }

    let file = open_log_file(log_dir).await?;

    let config = ConfigBuilder::new()
        .set_target_level(simplelog::LevelFilter::Debug)
        .set_location_level(simplelog::LevelFilter::Debug)
        .add_filter_ignore_str("tokio")
        .add_filter_ignore_str("hyper")
        .add_filter_ignore_str("sqlx")
        .add_filter_ignore_str("reqwest")
        .add_filter_ignore_str("h2")
        .build();

    simplelog::CombinedLogger::init(vec![
        simplelog::TermLogger::new(
            simplelog::LevelFilter::Debug,
            config,
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        ),
        simplelog::WriteLogger::new(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            file,
        ),
    ])?;

    Ok(())
}

fn get_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create_initial_tables",
            sql: r#"
                CREATE TABLE accounts (uid INTEGER, platform TEXT NOT NULL DEFAULT 'bilibili', name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT, PRIMARY KEY(uid, platform));
                CREATE TABLE recorders (room_id INTEGER PRIMARY KEY, platform TEXT NOT NULL DEFAULT 'bilibili', created_at TEXT);
                CREATE TABLE records (live_id TEXT PRIMARY KEY, platform TEXT NOT NULL DEFAULT 'bilibili', room_id INTEGER, title TEXT, length INTEGER, size INTEGER, cover BLOB, created_at TEXT);
                CREATE TABLE danmu_statistics (live_id TEXT PRIMARY KEY, room_id INTEGER, value INTEGER, time_point TEXT);
                CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, content TEXT, read INTEGER, created_at TEXT);
                CREATE TABLE videos (id INTEGER PRIMARY KEY AUTOINCREMENT, room_id INTEGER, cover TEXT, file TEXT, length INTEGER, size INTEGER, status INTEGER, bvid TEXT, title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT);
                "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "add_auto_start_column",
            sql: r#"ALTER TABLE recorders ADD COLUMN auto_start INTEGER NOT NULL DEFAULT 1;"#,
            kind: MigrationKind::Up,
        },
    ]
}

#[cfg(feature = "headless")]
#[derive(Debug)]
struct MigrationList(Vec<Migration>);

#[cfg(feature = "headless")]
impl MigrationSource<'static> for MigrationList {
    fn resolve(self) -> BoxFuture<'static, std::result::Result<Vec<SqlxMigration>, BoxDynError>> {
        Box::pin(async move {
            let mut migrations = Vec::new();
            for migration in self.0 {
                if matches!(migration.kind, MigrationKind::Up) {
                    migrations.push(SqlxMigration::new(
                        migration.version,
                        migration.description.into(),
                        migration.kind.into(),
                        migration.sql.into(),
                        false,
                    ));
                }
            }
            Ok(migrations)
        })
    }
}

#[cfg(feature = "headless")]
async fn setup_server_state(args: Args) -> Result<State, Box<dyn std::error::Error>> {
    use std::path::PathBuf;

    use progress_manager::ProgressManager;
    use progress_reporter::EventEmitter;

    setup_logging(Path::new("./")).await?;
    log::info!("Setting up server state...");
    let config_path = PathBuf::from(&args.config);
    let cache_path = PathBuf::from("./cache");
    let output_path = PathBuf::from("./output");
    let config = match Config::load(&config_path, &cache_path, &output_path) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Failed to load config: {e}");
            return Err(e.into());
        }
    };
    let client = Arc::new(BiliClient::new()?);
    let config = Arc::new(RwLock::new(config));
    let db = Arc::new(Database::new());
    // connect to sqlite database

    let conn_url = format!("sqlite:{}/data_v2.db", args.db);
    // create db folder if not exists
    if !Path::new(&args.db).exists() {
        std::fs::create_dir_all(&args.db)?;
    }

    if !Sqlite::database_exists(&conn_url).await.unwrap_or(false) {
        Sqlite::create_database(&conn_url).await?;
    }
    let db_pool: Pool<Sqlite> = Pool::connect(&conn_url).await?;
    let migrations = get_migrations();

    let migrator = Migrator::new(MigrationList(migrations))
        .await
        .expect("Failed to create migrator");
    migrator
        .run(&db_pool)
        .await
        .expect("Failed to run migrations");

    db.set(db_pool).await;

    let progress_manager = Arc::new(ProgressManager::new());
    let emitter = EventEmitter::new(progress_manager.get_event_sender());
    let recorder_manager = Arc::new(RecorderManager::new(emitter, db.clone(), config.clone()));
    let _ = try_rebuild_archives(&db, config.read().await.cache.clone().into()).await;

    Ok(State {
        db,
        client,
        config,
        recorder_manager,
        progress_manager,
    })
}

#[cfg(feature = "gui")]
async fn setup_app_state(app: &tauri::App) -> Result<State, Box<dyn std::error::Error>> {
    use platform_dirs::AppDirs;
    use progress_reporter::EventEmitter;

    let log_dir = app.path().app_log_dir()?;
    setup_logging(&log_dir).await?;

    log::info!("Setting up app state...");
    let app_dirs = AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false).unwrap();
    let config_path = app_dirs.config_dir.join("Conf.toml");
    let cache_path = app_dirs.cache_dir.join("cache");
    let output_path = app_dirs.data_dir.join("output");
    log::info!("Loading config from {:?}", config_path);
    let config = match Config::load(&config_path, &cache_path, &output_path) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Failed to load config, exiting: {e}");
            return Err(e.into());
        }
    };

    let client = Arc::new(BiliClient::new()?);
    let config = Arc::new(RwLock::new(config));
    let config_clone = config.clone();
    let dbs = app.state::<tauri_plugin_sql::DbInstances>().inner();
    let db = Arc::new(Database::new());
    let db_clone = db.clone();
    let client_clone = client.clone();
    let emitter = EventEmitter::new(app.handle().clone());
    let binding = dbs.0.read().await;
    let dbpool = binding.get("sqlite:data_v2.db").unwrap();
    let sqlite_pool = match dbpool {
        tauri_plugin_sql::DbPool::Sqlite(pool) => Some(pool),
    };
    db_clone.set(sqlite_pool.unwrap().clone()).await;

    let recorder_manager = Arc::new(RecorderManager::new(
        app.app_handle().clone(),
        emitter,
        db.clone(),
        config.clone(),
    ));

    let accounts = db_clone.get_accounts().await?;
    if accounts.is_empty() {
        log::warn!("No account found");
        return Ok(State {
            db,
            client,
            config,
            recorder_manager,
            app_handle: app.handle().clone(),
        });
    }

    // update account infos
    for account in accounts {
        // only update bilibili account
        let platform = PlatformType::from_str(&account.platform).unwrap();
        if platform != PlatformType::BiliBili {
            continue;
        }

        match client_clone.get_user_info(&account, account.uid).await {
            Ok(account_info) => {
                if let Err(e) = db_clone
                    .update_account(
                        &account.platform,
                        account_info.user_id,
                        &account_info.user_name,
                        &account_info.user_avatar_url,
                    )
                    .await
                {
                    log::error!("Error when updating account info {}", e);
                }
            }
            Err(e) => {
                log::error!("Get user info failed {}", e);
            }
        }
    }

    // try to rebuild archive table
    let cache_path = config_clone.read().await.cache.clone();
    if let Err(e) = try_rebuild_archives(&db_clone, cache_path.into()).await {
        log::warn!("Rebuilding archive table failed: {}", e);
    }

    Ok(State {
        db,
        client,
        config,
        recorder_manager,
        app_handle: app.handle().clone(),
    })
}

#[cfg(feature = "gui")]
fn setup_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let migrations = get_migrations();
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app
                .get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }))
        .plugin(tauri_plugin_sql::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:data_v2.db", migrations)
                .build(),
        )
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init());

    println!("Plugins initialized");

    builder
}

#[cfg(feature = "gui")]
fn setup_event_handlers(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.on_window_event(|window, event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            if !window.label().starts_with("Live") {
                window.hide().unwrap();
                api.prevent_close();
            }
        }
    })
}

#[cfg(feature = "gui")]
fn setup_invoke_handlers(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        crate::handlers::account::get_accounts,
        crate::handlers::account::add_account,
        crate::handlers::account::remove_account,
        crate::handlers::account::get_account_count,
        crate::handlers::account::get_qr_status,
        crate::handlers::account::get_qr,
        crate::handlers::config::get_config,
        crate::handlers::config::set_cache_path,
        crate::handlers::config::set_output_path,
        crate::handlers::config::update_notify,
        crate::handlers::config::update_whisper_model,
        crate::handlers::config::update_subtitle_setting,
        crate::handlers::config::update_clip_name_format,
        crate::handlers::config::update_whisper_prompt,
        crate::handlers::config::update_auto_generate,
        crate::handlers::message::get_messages,
        crate::handlers::message::read_message,
        crate::handlers::message::delete_message,
        crate::handlers::recorder::get_recorder_list,
        crate::handlers::recorder::add_recorder,
        crate::handlers::recorder::remove_recorder,
        crate::handlers::recorder::get_room_info,
        crate::handlers::recorder::get_archives,
        crate::handlers::recorder::get_archive,
        crate::handlers::recorder::delete_archive,
        crate::handlers::recorder::get_danmu_record,
        crate::handlers::recorder::export_danmu,
        crate::handlers::recorder::send_danmaku,
        crate::handlers::recorder::get_total_length,
        crate::handlers::recorder::get_today_record_count,
        crate::handlers::recorder::get_recent_record,
        crate::handlers::recorder::set_auto_start,
        crate::handlers::recorder::force_start,
        crate::handlers::recorder::force_stop,
        crate::handlers::recorder::fetch_hls,
        crate::handlers::video::clip_range,
        crate::handlers::video::upload_procedure,
        crate::handlers::video::cancel,
        crate::handlers::video::get_video,
        crate::handlers::video::get_videos,
        crate::handlers::video::delete_video,
        crate::handlers::video::get_video_typelist,
        crate::handlers::video::update_video_cover,
        crate::handlers::video::generate_video_subtitle,
        crate::handlers::video::get_video_subtitle,
        crate::handlers::video::update_video_subtitle,
        crate::handlers::video::encode_video_subtitle,
        crate::handlers::utils::show_in_folder,
        crate::handlers::utils::export_to_file,
        crate::handlers::utils::get_disk_info,
        crate::handlers::utils::open_live,
        crate::handlers::utils::open_log_folder,
    ])
}

#[cfg(feature = "gui")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = fix_path_env::fix();

    let builder = tauri::Builder::default();
    let builder = setup_plugins(builder);
    let builder = setup_event_handlers(builder);
    let builder = setup_invoke_handlers(builder);

    builder
        .setup(|app| {
            tauri::async_runtime::block_on(async {
                let state = setup_app_state(app).await?;
                let _ = tray::create_tray(app.handle());

                // check ffmpeg status
                match ffmpeg::check_ffmpeg().await {
                    Err(e) => log::error!("Failed to check ffmpeg version: {e}"),
                    Ok(v) => log::info!("Checked ffmpeg version: {v}"),
                }

                app.manage(state);
                Ok(())
            })
        })
        .run(tauri::generate_context!())?;

    Ok(())
}

#[cfg(feature = "headless")]
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the config file
    #[arg(short, long, default_value_t = String::from("config.toml"))]
    config: String,

    /// Path to the database folder
    #[arg(short, long, default_value_t = String::from("./data"))]
    db: String,
}

#[cfg(feature = "headless")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // get params from command line
    let args = Args::parse();
    let state = setup_server_state(args)
        .await
        .expect("Failed to setup server state");

    // check ffmpeg status
    match ffmpeg::check_ffmpeg().await {
        Err(e) => log::error!("Failed to check ffmpeg version: {e}"),
        Ok(v) => log::info!("Checked ffmpeg version: {v}"),
    }

    http_server::start_api_server(state).await;
    Ok(())
}
