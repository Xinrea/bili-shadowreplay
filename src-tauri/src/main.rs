// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive_migration;
mod config;
mod danmu2ass;
mod database;
mod ffmpeg;
mod handlers;
mod progress_event;
mod recorder;
mod recorder_manager;
mod state;
mod subtitle_generator;
mod tray;

use archive_migration::try_rebuild_archives;
use config::Config;
use database::Database;
use recorder::{bilibili::client::BiliClient, PlatformType};
use recorder_manager::RecorderManager;
use simplelog::ConfigBuilder;
use state::State;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use tauri::{Manager, WindowEvent};
use tauri_plugin_sql::{Migration, MigrationKind};
use tokio::sync::RwLock;

async fn setup_logging(log_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // mkdir if not exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)?;
    }

    let log_file = log_dir.join("bsr.log");

    // open file with append mode
    let file = File::options().create(true).append(true).open(&log_file)?;

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

async fn setup_app_state(app: &tauri::App) -> Result<State, Box<dyn std::error::Error>> {
    println!("Setting up app state...");
    let client = Arc::new(BiliClient::new()?);
    let config = Arc::new(RwLock::new(Config::load()));
    let config_clone = config.clone();
    let dbs = app.state::<tauri_plugin_sql::DbInstances>().inner();
    let db = Arc::new(Database::new());
    let db_clone = db.clone();
    let client_clone = client.clone();

    let log_dir = app.path().app_log_dir()?;
    setup_logging(&log_dir).await?;

    let recorder_manager = Arc::new(RecorderManager::new(
        app.handle().clone(),
        db.clone(),
        config.clone(),
    ));
    let recorder_manager_clone = recorder_manager.clone();
    let binding = dbs.0.read().await;
    let dbpool = binding.get("sqlite:data_v2.db").unwrap();
    let sqlite_pool = match dbpool {
        tauri_plugin_sql::DbPool::Sqlite(pool) => Some(pool),
    };
    db_clone.set(sqlite_pool.unwrap().clone()).await;

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

    let bili_account = db_clone.get_account_by_platform("bilibili").await;

    if let Ok(bili_account) = bili_account {
        let mut webid = client_clone.fetch_webid(&bili_account).await;
        if webid.is_err() {
            log::error!("Failed to fetch webid: {}", webid.err().unwrap());
            webid = Ok("".to_string());
        }

        let webid = webid.unwrap();

        // update account infos
        for account in accounts {
            // only update bilibili account
            let platform = PlatformType::from_str(&account.platform).unwrap();
            if platform != PlatformType::BiliBili {
                continue;
            }

            match client_clone
                .get_user_info(&webid, &account, account.uid)
                .await
            {
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

                // only auto download ffmpeg if it's linux
                if cfg!(target_os = "linux") {
                    if let Err(e) = async_ffmpeg_sidecar::download::auto_download().await {
                        log::error!("Error when auto downloading ffmpeg: {}", e);
                    }
                }

                log::info!(
                    "FFMPEG version: {:?}",
                    async_ffmpeg_sidecar::version::ffmpeg_version().await
                );

                app.manage(state);
                Ok(())
            })
        })
        .run(tauri::generate_context!())?;

    Ok(())
}
