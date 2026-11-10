//! Dashboard capacity is grouped by proven shared-space identity, never by
//! equal capacity, a volume label or the physical disk holding a partition.
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct StoragePoolInfo {
    pub id: String,
    pub name: String,
    pub mount_points: Vec<String>,
    pub total: u64,
    pub free: u64,
}

struct MountedDisk {
    mount: String,
    name: String,
    total: u64,
    free: u64,
}

struct PoolIdentity {
    id: String,
    name: String,
    total: u64,
    free: u64,
}

pub async fn local_storage_pools() -> Result<Vec<StoragePoolInfo>, ()> {
    // sysinfo excludes network drives and most pseudo filesystems by default.
    let disks = tokio::task::spawn_blocking(|| {
        sysinfo::Disks::new_with_refreshed_list()
            .iter()
            .map(|disk| MountedDisk {
                mount: disk.mount_point().to_string_lossy().into_owned(),
                name: disk.name().to_string_lossy().into_owned(),
                total: disk.total_space(),
                free: disk.available_space(),
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| log::error!("Failed to enumerate local disks: {error}"))?;

    #[cfg(target_os = "macos")]
    let resolved = {
        use futures::{stream, StreamExt};
        // Bound both the number and lifetime of diskutil processes. Failure to
        // identify one mount must not prevent reporting other local capacity.
        stream::iter(disks)
            .map(|disk| async move {
                let identity = macos::pool_identity(&disk.mount).await;
                (disk, identity)
            })
            .buffered(4)
            .collect()
            .await
    };
    // Other platforms retain independent filesystems until a reliable pool
    // provider is available. In particular, Windows volume labels are not IDs.
    #[cfg(not(target_os = "macos"))]
    let resolved = disks.into_iter().map(|disk| (disk, None)).collect();
    Ok(group_pools(resolved))
}

fn group_pools(mut disks: Vec<(MountedDisk, Option<PoolIdentity>)>) -> Vec<StoragePoolInfo> {
    disks.retain(|(disk, _)| !disk.mount.is_empty() && disk.total > 0);
    disks.sort_by(|(left, _), (right, _)| left.mount.cmp(&right.mount));
    disks.dedup_by(|(left, _), (right, _)| left.mount == right.mount);
    let mut pools = BTreeMap::<String, StoragePoolInfo>::new();
    for (disk, identity) in disks {
        let identity = identity
            .filter(|pool| !pool.id.is_empty() && pool.total > 0 && pool.free <= pool.total);
        let (id, name, total, free) = match identity {
            Some(pool) => (pool.id, pool.name, pool.total, pool.free),
            None => (
                format!("mount:{}", disk.mount),
                disk.name,
                disk.total,
                disk.free.min(disk.total),
            ),
        };
        let name = if name.trim().is_empty() {
            disk.mount.clone()
        } else {
            name
        };
        let pool = pools.entry(id.clone()).or_insert_with(|| StoragePoolInfo {
            id,
            // Mount ordering prefers "/" and therefore the system volume's
            // "Macintosh HD" name over "Data". Names are labels, not identity.
            name,
            mount_points: Vec::new(),
            total,
            free,
        });
        pool.mount_points.push(disk.mount);
        // Measurements may differ slightly between calls. Keep a conservative
        // free-space snapshot, never add capacity or volume usage together.
        pool.free = pool.free.min(free).min(pool.total);
    }
    let mut pools: Vec<_> = pools.into_values().collect();
    pools.sort_by(|left, right| left.mount_points[0].cmp(&right.mount_points[0]));
    pools
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PoolIdentity;

    #[derive(serde::Deserialize)]
    struct VolumeInfo {
        #[serde(rename = "FilesystemType")]
        filesystem: Option<String>,
        #[serde(rename = "APFSContainerReference")]
        container: Option<String>,
        #[serde(rename = "APFSContainerSize")]
        total: Option<u64>,
        #[serde(rename = "APFSContainerFree")]
        free: Option<u64>,
        #[serde(rename = "VolumeName")]
        name: Option<String>,
    }

    pub(super) async fn pool_identity(mount: &str) -> Option<PoolIdentity> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::process::Command::new("/usr/sbin/diskutil")
                .args(["info", "-plist", mount])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_identity(&output.stdout)
    }

    fn parse_identity(bytes: &[u8]) -> Option<PoolIdentity> {
        let info: VolumeInfo = plist::from_bytes(bytes).ok()?;
        if info.filesystem.as_deref() != Some("apfs") {
            return None;
        }
        let container = info.container.filter(|value| !value.trim().is_empty())?;
        let total = info.total.filter(|total| *total > 0)?;
        let free = info.free.filter(|free| *free <= total)?;
        Some(PoolIdentity {
            id: format!("apfs:{container}"),
            name: info.name.unwrap_or_default(),
            total,
            free,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn apfs_identity_uses_container_capacity_not_volume_or_physical_disk() {
            let xml = br#"<?xml version="1.0"?><plist version="1.0"><dict>
                <key>FilesystemType</key><string>apfs</string>
                <key>APFSContainerReference</key><string>disk3</string>
                <key>APFSContainerSize</key><integer>1000</integer>
                <key>APFSContainerFree</key><integer>80</integer>
                <key>VolumeName</key><string>Macintosh HD</string>
                <key>DeviceIdentifier</key><string>disk3s1s1</string>
                <key>ParentWholeDisk</key><string>disk3</string>
            </dict></plist>"#;
            let identity = parse_identity(xml).unwrap();
            assert_eq!(identity.id, "apfs:disk3");
            assert_eq!(identity.name, "Macintosh HD");
            assert_eq!((identity.total, identity.free), (1000, 80));
            assert!(parse_identity(b"not a plist").is_none());
            let text = std::str::from_utf8(xml).unwrap();
            for invalid in [
                text.replace("<string>apfs</string>", "<string>hfs</string>"),
                text.replace("<string>disk3</string>", "<string></string>"),
                text.replace("<integer>80</integer>", "<integer>2000</integer>"),
                text.replace("<key>APFSContainerFree</key><integer>80</integer>", ""),
            ] {
                assert!(parse_identity(invalid.as_bytes()).is_none());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(mount: &str) -> MountedDisk {
        MountedDisk {
            mount: mount.into(),
            name: mount.into(),
            total: 100,
            free: 20,
        }
    }

    fn apfs(id: &str, name: &str, free: u64) -> Option<PoolIdentity> {
        Some(PoolIdentity {
            id: id.into(),
            name: name.into(),
            total: 1000,
            free,
        })
    }

    #[test]
    fn system_snapshot_and_data_volume_share_one_pool_without_summing() {
        let pools = group_pools(vec![
            (disk("/System/Volumes/Data"), apfs("apfs:disk3", "Data", 81)),
            (disk("/"), apfs("apfs:disk3", "Macintosh HD", 80)),
        ]);
        assert_eq!(
            pools,
            vec![StoragePoolInfo {
                id: "apfs:disk3".into(),
                name: "Macintosh HD".into(),
                mount_points: vec!["/".into(), "/System/Volumes/Data".into()],
                total: 1000,
                free: 80,
            }]
        );
    }

    #[test]
    fn equal_labels_and_capacities_do_not_merge_distinct_pools_or_unknown_mounts() {
        let pools = group_pools(vec![
            (disk("/a"), apfs("apfs:disk1", "Archive", 80)),
            (disk("/b"), apfs("apfs:disk2", "Archive", 80)),
            (disk("/c"), None),
            (disk("/d"), None),
        ]);
        assert_eq!(pools.len(), 4);
        assert_eq!(pools[0].name, pools[1].name);
        assert_ne!(pools[0].id, pools[1].id);
        assert_eq!(pools[2].id, "mount:/c");
    }

    #[test]
    fn missing_metadata_does_not_hide_mounts_and_invalid_sizes_are_safe() {
        let pools = group_pools(vec![
            (disk("/"), None),
            (disk("/System/Volumes/Data"), apfs("apfs:disk3", "Data", 80)),
            (
                MountedDisk {
                    name: String::new(),
                    free: 200,
                    ..disk("/external")
                },
                None,
            ),
            (
                MountedDisk {
                    total: 0,
                    ..disk("/empty")
                },
                None,
            ),
            (disk(""), None),
        ]);
        assert_eq!(pools.len(), 3);
        assert_eq!(pools[2].name, "/external");
        assert_eq!((pools[2].total, pools[2].free), (100, 100));
        assert!(group_pools(Vec::new()).is_empty());
    }
}
