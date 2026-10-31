//! Strict capability result, separate from the model's actual execution evidence.
use super::*;
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) struct Selection {
    pub(super) physical_device: Option<u32>,
    pub(super) raw: PathBuf,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
enum Capability {
    #[serde(rename_all = "camelCase")]
    Candidate {
        schema_version: u32,
        physical_device_index: u32,
        vendor_id: u32,
        device_id: u32,
        api_version: u32,
        driver_version: u32,
        device_type: u32,
        storage_buffer16_bit_access: bool,
        compute_queue: bool,
    },
    #[serde(rename_all = "camelCase")]
    Unavailable { schema_version: u32, reason: String },
}

fn selected(raw: &[u8]) -> Result<Option<u32>, String> {
    match serde_json::from_slice::<Capability>(raw).map_err(|_| failure())? {
        Capability::Candidate {
            schema_version: 1,
            physical_device_index,
            api_version,
            device_type,
            storage_buffer16_bit_access: true,
            compute_queue: true,
            vendor_id,
            device_id,
            driver_version,
        } if physical_device_index < 256
            && api_version >= ((1 << 22) | (2 << 12))
            && matches!(device_type, 1 | 2) =>
        {
            // Identity fields are retained in the raw probe, not used to infer support.
            let _ = (vendor_id, device_id, driver_version);
            Ok(Some(physical_device_index))
        }
        Capability::Unavailable {
            schema_version: 1,
            reason,
        } if matches!(
            reason.as_str(),
            "loader-missing" | "vulkan-version" | "incompatible-driver" | "no-supported-device"
        ) =>
        {
            Ok(None)
        }
        _ => Err(failure()),
    }
}

pub(super) fn select(
    probe: &Path,
    work: &Path,
    cancel: &AtomicBool,
    inherited_tree: bool,
) -> Result<Selection, String> {
    cancelled(cancel)?;
    let raw = work.join("whisper-vulkan-probe.json");
    let stderr = work.join("whisper-vulkan-probe.stderr.log");
    let mut command = Command::new(probe);
    command
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&raw)
                .map_err(|e| e.to_string())?,
        )
        .stderr(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(stderr)
                .map_err(|e| e.to_string())?,
        );
    for name in [
        "VK_INSTANCE_LAYERS",
        "VK_LAYER_PATH",
        "VK_ADD_LAYER_PATH",
        "VK_ICD_FILENAMES",
        "VK_DRIVER_FILES",
        "VK_ADD_DRIVER_FILES",
    ] {
        command.env_remove(name);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000);
    }
    let mut child = if inherited_tree {
        crate::process_tree::spawn_inherited(&mut command)
    } else {
        crate::process_tree::spawn(&mut command)
    }?;
    let began = Instant::now();
    loop {
        cancelled(cancel)?;
        if began.elapsed() > Duration::from_secs(30) {
            return Err(failure());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(failure());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(Selection {
        physical_device: selected(&read(&raw, 64 * 1024)?)?,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_requires_features_and_known_unavailability_never_allocation_failure() {
        let mut candidate = json!({"schemaVersion":1,"status":"candidate","physicalDeviceIndex":0,
            "vendorId":4318,"deviceId":1,"apiVersion":4202496,"driverVersion":1,"deviceType":2,
            "storageBuffer16BitAccess":true,"computeQueue":true});
        assert_eq!(
            selected(&serde_json::to_vec(&candidate).unwrap()).unwrap(),
            Some(0)
        );
        candidate["storageBuffer16BitAccess"] = false.into();
        assert!(selected(&serde_json::to_vec(&candidate).unwrap()).is_err());
        for reason in [
            "loader-missing",
            "vulkan-version",
            "incompatible-driver",
            "no-supported-device",
        ] {
            assert_eq!(
                selected(
                    &serde_json::to_vec(
                        &json!({"schemaVersion":1,"status":"unavailable","reason":reason})
                    )
                    .unwrap()
                )
                .unwrap(),
                None
            );
        }
        for reason in ["out-of-memory", "driver-error", "probe-timeout", "failed"] {
            assert!(selected(
                &serde_json::to_vec(
                    &json!({"schemaVersion":1,"status":"unavailable","reason":reason})
                )
                .unwrap()
            )
            .is_err());
        }
    }
}
