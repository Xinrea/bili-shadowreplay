use std::path::PathBuf;

use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use {
    crate::recorder::PlatformType,
    std::process::Command,
    tauri::State as TauriState,
    tauri::{Manager, Theme},
    tauri_utils::config::WindowEffectsConfig,
    tokio::fs::OpenOptions,
    tokio::io::AsyncWriteExt,
};

pub fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .args(["/select,", &path]) // The comma after select is not a typo
            .spawn()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::metadata;
        use std::path::PathBuf;
        if path.contains(",") {
            // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
            let new_path = match metadata(&path).unwrap().is_dir() {
                true => path,
                false => {
                    let mut path2 = PathBuf::from(path);
                    path2.pop();
                    path2.into_os_string().into_string().unwrap()
                }
            };
            Command::new("xdg-open").arg(&new_path).spawn().unwrap();
        } else {
            Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    format!("array:string:\"file://{path}\"").as_str(),
                    "string:\"\"",
                ])
                .spawn()
                .unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path])
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
    }
}

#[derive(serde::Serialize)]
pub struct DiskInfo {
    disk: String,
    total: u64,
    free: u64,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_disk_info(state: state_type!()) -> Result<DiskInfo, ()> {
    let cache = state.config.read().await.cache.clone();
    // if cache is relative path, convert it to absolute path
    let mut cache = PathBuf::from(&cache);
    if cache.is_relative() {
        // get current working directory
        let cwd = std::env::current_dir().unwrap();
        cache = cwd.join(cache);
    }
    #[cfg(target_os = "linux")]
    {
        // get disk info from df command
        let output = tokio::process::Command::new("df")
            .arg(cache)
            .output()
            .await
            .unwrap();
        let output_str = String::from_utf8(output.stdout).unwrap();
        // Filesystem     1K-blocks     Used Available Use% Mounted on
        // /dev/nvme0n1p2 959218776 43826092 866593352   5% /app/cache
        let lines = output_str.lines().collect::<Vec<&str>>();
        if lines.len() < 2 {
            log::error!("df command output is too short: {}", output_str);
            return Err(());
        }
        let parts = lines[1].split_whitespace().collect::<Vec<&str>>();
        let disk = parts[0].to_string();
        let total = parts[1].parse::<u64>().unwrap() * 1024;
        let free = parts[3].parse::<u64>().unwrap() * 1024;

        return Ok(DiskInfo { disk, total, free });
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        // check system disk info
        let disks = sysinfo::Disks::new_with_refreshed_list();
        // get cache disk info
        let mut disk_info = DiskInfo {
            disk: "".into(),
            total: 0,
            free: 0,
        };

        // Find the disk with the longest matching mount point
        let mut longest_match = 0;
        for disk in disks.list() {
            let mount_point = disk.mount_point().to_str().unwrap();
            if cache.starts_with(mount_point) && mount_point.split("/").count() > longest_match {
                disk_info.disk = mount_point.into();
                disk_info.total = disk.total_space();
                disk_info.free = disk.available_space();
                longest_match = mount_point.split("/").count();
            }
        }

        Ok(disk_info)
    }
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn export_to_file(
    _state: state_type!(),
    file_name: &str,
    content: &str,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_name)
        .await;
    if file.is_err() {
        return Err(format!("Open file failed: {}", file.err().unwrap()));
    }
    let mut file = file.unwrap();
    if let Err(e) = file.write_all(content.as_bytes()).await {
        return Err(format!("Write file failed: {}", e));
    }
    if let Err(e) = file.flush().await {
        return Err(format!("Flush file failed: {}", e));
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_log_folder(state: state_type!()) -> Result<(), String> {
    #[cfg(feature = "gui")]
    {
        let log_dir = state.app_handle.path().app_log_dir().unwrap();
        show_in_folder(log_dir.to_str().unwrap().to_string());
    }
    Ok(())
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn open_live(
    state: state_type!(),
    platform: String,
    room_id: u64,
    live_id: String,
) -> Result<(), String> {
    log::info!("Open player window: {} {}", room_id, live_id);
    #[cfg(feature = "gui")]
    {
        let platform = PlatformType::from_str(&platform).unwrap();
        let recorder_info = state
            .recorder_manager
            .get_recorder_info(platform, room_id)
            .await
            .unwrap();
        let builder = tauri::WebviewWindowBuilder::new(
            &state.app_handle,
            format!("Live:{}:{}", room_id, live_id),
            tauri::WebviewUrl::App(
                format!(
                    "live_index.html?platform={}&room_id={}&live_id={}",
                    platform.as_str(),
                    room_id,
                    live_id
                )
                .into(),
            ),
        )
        .title(format!(
            "Live[{}] {}",
            room_id, recorder_info.room_info.room_title
        ))
        .theme(Some(Theme::Light))
        .inner_size(1200.0, 800.0)
        .effects(WindowEffectsConfig {
            effects: vec![
                tauri_utils::WindowEffect::Tabbed,
                tauri_utils::WindowEffect::Mica,
            ],
            state: None,
            radius: None,
            color: None,
        });

        if let Err(e) = builder.decorations(true).build() {
            log::error!("live window build failed: {}", e);
        }
    }

    Ok(())
}
