// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod database;
mod ffmpeg;
mod handlers;
mod progress_event;
mod recorder;
mod recorder_manager;
mod state;
mod subtitle_generator;
mod tray;

use config::Config;
use database::Database;
use recorder::{bilibili::client::BiliClient, PlatformType};
use recorder_manager::RecorderManager;
use state::State;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use tauri::{Manager, WindowEvent};
use tauri_plugin_sql::{Migration, MigrationKind};
use tokio::sync::RwLock;

fn setup_logging(log_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // mkdir if not exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)?;
    }

    let log_file = log_dir.join("bsr.log");

    // open file with append mode
    let file = File::options().create(true).append(true).open(&log_file)?;

    simplelog::CombinedLogger::init(vec![
        simplelog::TermLogger::new(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
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
    let client = Arc::new(BiliClient::new()?);
    let config = Arc::new(RwLock::new(Config::load()));
    let config_clone = config.clone();
    let dbs = app.state::<tauri_plugin_sql::DbInstances>().inner();
    let db = Arc::new(Database::new());
    let db_clone = db.clone();
    let client_clone = client.clone();

    let log_dir = app.path().app_log_dir()?;
    setup_logging(&log_dir)?;

    let recorder_manager = Arc::new(RecorderManager::new(
        app.handle().clone(),
        db.clone(),
        config.clone(),
    ));
    let recorder_manager_clone = recorder_manager.clone();

    let _ = recorder_manager_clone.run_hls().await;
    let binding = dbs.0.lock().await;
    let dbpool = binding.get("sqlite:data_v2.db").unwrap();
    let sqlite_pool = match dbpool {
        tauri_plugin_sql::DbPool::Sqlite(pool) => Some(pool),
    };
    db_clone.set(sqlite_pool.unwrap().clone()).await;
    let mut primary_uid = config_clone.read().await.primary_uid;
    let accounts = db_clone.get_accounts().await?;
    if accounts.is_empty() {
        log::warn!("No account found");
        return Ok(State {
            db,
            client,
            config,
            recorder_manager,
            app_handle: app.handle().clone(),
            cancel_flag_map: Arc::new(RwLock::new(std::collections::HashMap::new())),
        });
    }

    let mut primary_account = accounts.first().unwrap().clone();
    if primary_uid == 0 {
        primary_uid = primary_account.uid;
        config_clone.write().await.primary_uid = primary_uid;
        config_clone.write().await.save();
    }

    match accounts.iter().find(|x| x.uid == primary_uid) {
        Some(account) => {
            primary_account = account.clone();
        }
        None => {
            log::warn!("Primary account not found, using first account");
            primary_uid = primary_account.uid;
            config_clone.write().await.primary_uid = primary_uid;
            config_clone.write().await.save();
        }
    }

    let webid = client_clone.fetch_webid(&primary_account).await?;
    config_clone.write().await.webid = webid.clone();
    config_clone.write().await.webid_ts = chrono::Utc::now().timestamp();

    // update account infos
    for account in accounts {
        // only update bilibili account
        let platform = PlatformType::from_str(&account.platform).unwrap();
        if platform != PlatformType::BiliBili {
            continue;
        }

        match client_clone
            .get_user_info(&webid, &primary_account, account.uid)
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

    init_rooms(db_clone, recorder_manager_clone, &primary_account, &webid).await;

    Ok(State {
        db,
        client,
        config,
        recorder_manager,
        app_handle: app.handle().clone(),
        cancel_flag_map: Arc::new(RwLock::new(std::collections::HashMap::new())),
    })
}

async fn init_rooms(
    db_clone: Arc<Database>,
    recorder_manager_clone: Arc<RecorderManager>,
    primary_account: &database::account::AccountRow,
    webid: &str,
) {
    let initial_rooms = db_clone.get_recorders().await.unwrap();
    for room in &initial_rooms {
        let platform = PlatformType::from_str(&room.platform).unwrap();
        if platform != PlatformType::BiliBili {
            continue;
        }
        if let Err(e) = recorder_manager_clone
            .add_recorder(
                webid,
                primary_account,
                platform,
                room.room_id,
                room.auto_start,
            )
            .await
        {
            log::error!("error when adding initial rooms: {}", e);
        }
    }

    match db_clone.get_account_by_platform("douyin").await {
        Ok(account) => {
            for room in &initial_rooms {
                if room.platform != "douyin" {
                    continue;
                }
                if let Err(e) = recorder_manager_clone
                    .add_recorder(
                        webid,
                        &account,
                        PlatformType::Douyin,
                        room.room_id,
                        room.auto_start,
                    )
                    .await
                {
                    log::error!("error when adding initial rooms: {}", e);
                }
            }
        }
        Err(e) => {
            log::warn!("No available douyin account found: {}", e);
        }
    }
}

fn setup_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let migrations = get_migrations();
    builder
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
        .plugin(tauri_plugin_dialog::init())
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
        crate::handlers::account::set_primary,
        crate::handlers::account::get_qr_status,
        crate::handlers::account::get_qr,
        crate::handlers::config::get_config,
        crate::handlers::config::set_cache_path,
        crate::handlers::config::set_output_path,
        crate::handlers::config::update_notify,
        crate::handlers::config::update_whisper_model,
        crate::handlers::config::update_subtitle_setting,
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
        crate::handlers::recorder::send_danmaku,
        crate::handlers::recorder::get_total_length,
        crate::handlers::recorder::get_today_record_count,
        crate::handlers::recorder::get_recent_record,
        crate::handlers::recorder::set_auto_start,
        crate::handlers::recorder::force_start,
        crate::handlers::recorder::force_stop,
        crate::handlers::video::clip_range,
        crate::handlers::video::upload_procedure,
        crate::handlers::video::cancel_upload,
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
    // only auto download ffmpeg if it's not macOS
    if !cfg!(target_os = "macos") {
        ffmpeg_sidecar::download::auto_download().unwrap();
    }

    let builder = tauri::Builder::default();
    let builder = setup_plugins(builder);
    let builder = setup_event_handlers(builder);
    let builder = setup_invoke_handlers(builder);

    builder
        .setup(|app| {
            tauri::async_runtime::block_on(async {
                let state = setup_app_state(app).await?;
                let _ = tray::create_tray(app.handle());
                app.manage(state);
                Ok(())
            })
        })
        .run(tauri::generate_context!())?;

    Ok(())
}
