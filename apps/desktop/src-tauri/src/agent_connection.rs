use fs2::FileExt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CLIENTS: [(&str, &str); 2] = [("claude", "Claude"), ("codex", "Codex")];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    available: bool,
    executable_path: Option<String>,
    data_dir: String,
    config: Value,
    cli_example: String,
    clients: Vec<ClientInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    id: String,
    name: String,
    state: String,
    detail: Option<String>,
}

pub struct ConnectionAction {
    pub kind: String,
    pub target: String,
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "nero.exe"
    } else {
        "nero"
    }
}

pub fn connection_info(resources: &Path, root: &Path, connections: &Path) -> ConnectionInfo {
    let executable = resource_agent_path(resources);
    let binary = bounded_read(&executable, 64 * 1024 * 1024).ok();
    let available = binary.is_some();
    let current_generation = binary
        .as_ref()
        .and_then(|bytes| package_generation(resources, bytes).ok());
    let path = executable.to_string_lossy().into_owned();
    let data_dir = root.to_string_lossy().into_owned();
    let config = if available {
        json!({"mcpServers": {"nero": {"command": path, "args": ["--data-dir", data_dir, "mcp"]}}})
    } else {
        Value::Null
    };
    let cli_example = if available {
        format!(
            "{}{} --data-dir {} meetings list --json",
            if cfg!(windows) { "& " } else { "" },
            quote(&executable, cfg!(windows)),
            quote(root, cfg!(windows))
        )
    } else {
        String::new()
    };
    let templates = resources.join("agent/integrations");
    let clients = CLIENTS
        .iter()
        .map(|(id, name)| {
            let prepared = load_registration(&connections.join(id), root, id).ok();
            let state = if !available || !templates.is_dir() || current_generation.is_none() {
                "unavailable"
            } else if prepared.as_ref().is_some_and(|registration| {
                live_session(
                    &connections.join(id).join("sessions"),
                    id,
                    registration["token"].as_str().unwrap_or(""),
                    current_generation.as_deref().unwrap_or(""),
                )
            }) {
                "connected"
            } else if prepared.is_some() {
                "waiting"
            } else {
                "not_connected"
            };
            ClientInfo {
                id: (*id).into(),
                name: (*name).into(),
                state: state.into(),
                detail: prepared
                    .as_ref()
                    .filter(|registration| {
                        registration["generation"].as_str() != current_generation.as_deref()
                    })
                    .map(|_| "연결을 업데이트해 주세요.".into()),
            }
        })
        .collect();
    ConnectionInfo {
        available,
        executable_path: available.then_some(path),
        data_dir,
        config,
        cli_example,
        clients,
    }
}

pub fn prepare_connection(
    resources: &Path,
    root: &Path,
    connections: &Path,
    client: &str,
) -> Result<ConnectionAction, String> {
    if !CLIENTS.iter().any(|(id, _)| *id == client) {
        return Err("지원하지 않는 연결입니다.".into());
    }
    if !root.is_absolute() || !connections.is_absolute() {
        return Err("연결 경로를 확인하지 못했습니다.".into());
    }
    let templates = resources.join("agent/integrations");
    let binary = bounded_read(&resource_agent_path(resources), 64 * 1024 * 1024)
        .map_err(|_| "연결 도구가 포함되지 않은 빌드입니다.")?;
    if binary.is_empty() {
        return Err("연결 도구가 포함되지 않은 빌드입니다.".into());
    }
    let generation = package_generation(resources, &binary)?;
    let client_directory = connections.join(client);
    private_directory(&client_directory).map_err(|_| "연결 설정을 준비하지 못했습니다.")?;
    let preparation_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(client_directory.join("prepare.lock"))
        .map_err(|_| "연결을 준비하지 못했습니다.")?;
    preparation_lock
        .try_lock_exclusive()
        .map_err(|_| "연결을 준비 중입니다. 잠시 후 다시 시도해 주세요.")?;
    let state_path = client_directory.join("state.json");
    let mut registration = if state_path.exists() {
        load_registration(&client_directory, root, client)
            .map_err(|_| "기존 연결 경로를 확인하지 못했습니다.")?
    } else {
        let value = json!({"schemaVersion":1,"client":client,"token":uuid::Uuid::new_v4().to_string(),"dataDir":root,"generation":generation});
        atomic_write(&state_path, value.to_string().as_bytes())
            .map_err(|_| "연결 설정을 저장하지 못했습니다.")?;
        value
    };
    let sessions = client_directory.join("sessions");
    private_directory(&sessions).map_err(|_| "연결 확인을 준비하지 못했습니다.")?;
    registration["generation"] = json!(generation);
    atomic_write(&state_path, registration.to_string().as_bytes())
        .map_err(|_| "연결 업데이트를 저장하지 못했습니다.")?;
    let release = client_directory.join("releases").join(&generation);
    private_directory(&release).map_err(|_| "연결 도구를 준비하지 못했습니다.")?;
    let args = json!(["--data-dir", root, "mcp"]);
    let environment = json!({"NERO_CONNECTION_DIR":sessions,"NERO_CONNECTION_CLIENT":client,"NERO_CONNECTION_TOKEN":registration["token"],"NERO_CONNECTION_GENERATION":generation});
    let icon = bounded_read(&templates.join("icon.png"), 4 * 1024 * 1024)
        .map_err(|_| "연결 아이콘을 찾지 못했습니다.")?;
    let skill = bounded_read(&templates.join("meeting-summary/SKILL.md"), 128 * 1024)
        .map_err(|_| "회의 안내를 찾지 못했습니다.")?;
    if client == "claude" {
        let mut manifest = read_json(&templates.join("claude-manifest.json"))?;
        manifest["server"]["mcp_config"]["args"] = args;
        manifest["server"]["mcp_config"]["env"] = environment;
        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|_| "연결 파일을 만들지 못했습니다.")?;
        let package = stored_zip(&[
            ("manifest.json", &manifest_bytes, 0o644),
            (&format!("server/{}", executable_name()), &binary, 0o755),
            ("icon.png", &icon, 0o644),
        ])?;
        let path = release.join("Nero.mcpb");
        atomic_write(&path, &package).map_err(|_| "Claude 연결 파일을 저장하지 못했습니다.")?;
        return Ok(ConnectionAction {
            kind: "file".into(),
            target: path.to_string_lossy().into_owned(),
        });
    }
    let plugin = release.join("plugins/nero");
    private_directory(&plugin.join(".codex-plugin"))
        .map_err(|_| "Codex 연결을 준비하지 못했습니다.")?;
    private_directory(&plugin.join("skills/meeting-summary"))
        .map_err(|_| "Codex 회의 안내를 준비하지 못했습니다.")?;
    for (source, destination) in [
        ("codex-plugin.json", "plugin.json"),
        ("codex-legacy-plugin.json", ".codex-plugin/plugin.json"),
    ] {
        let template = bounded_read(&templates.join(source), 128 * 1024)
            .map_err(|_| "Codex 연결 파일을 찾지 못했습니다.")?;
        atomic_write(&plugin.join(destination), &template)
            .map_err(|_| "Codex 연결 파일을 저장하지 못했습니다.")?;
    }
    private_directory(&plugin.join("server"))
        .map_err(|_| "Codex 실행 경로를 준비하지 못했습니다.")?;
    let executable = plugin.join("server").join(executable_name());
    immutable_write(&executable, &binary).map_err(|_| "Codex 연결 도구를 저장하지 못했습니다.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .map_err(|_| "Codex 연결 도구를 실행할 수 없습니다.")?;
    }
    let config =
        json!({"mcpServers":{"nero":{"command":executable,"args":args,"env":environment}}});
    atomic_write(&plugin.join(".mcp.json"), config.to_string().as_bytes())
        .map_err(|_| "Codex 연결을 저장하지 못했습니다.")?;
    let portable = json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"nero":{"type":"stdio","command":format!("./server/{}", executable_name()),"args":config["mcpServers"]["nero"]["args"],"env":config["mcpServers"]["nero"]["env"]}}});
    atomic_write(&plugin.join("mcp.json"), portable.to_string().as_bytes())
        .map_err(|_| "Codex 연결을 저장하지 못했습니다.")?;
    atomic_write(&plugin.join("skills/meeting-summary/SKILL.md"), &skill)
        .map_err(|_| "Codex 회의 안내를 저장하지 못했습니다.")?;
    private_directory(&plugin.join("assets"))
        .map_err(|_| "Codex 아이콘 경로를 준비하지 못했습니다.")?;
    atomic_write(&plugin.join("assets/icon.png"), &icon)
        .map_err(|_| "Codex 아이콘을 저장하지 못했습니다.")?;
    let mut marketplace = read_json(&templates.join("codex-marketplace.json"))?;
    marketplace["plugins"][0]["source"]["path"] = json!("./plugins/nero");
    let marketplace_directory = release.join(".agents/plugins");
    private_directory(&marketplace_directory)
        .map_err(|_| "Codex 설치 경로를 준비하지 못했습니다.")?;
    let marketplace_path = marketplace_directory.join("marketplace.json");
    atomic_write(
        &marketplace_path,
        serde_json::to_string_pretty(&marketplace)
            .map_err(|_| "Codex 설치 정보를 만들지 못했습니다.")?
            .as_bytes(),
    )
    .map_err(|_| "Codex 설치 정보를 저장하지 못했습니다.")?;
    Ok(ConnectionAction {
        kind: "url".into(),
        target: format!(
            "codex://plugins/nero?marketplacePath={}",
            percent_encode(&marketplace_path.to_string_lossy())
        ),
    })
}

fn package_generation(resources: &Path, binary: &[u8]) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"nero-ai-package-v2\0");
    digest.update((binary.len() as u64).to_le_bytes());
    digest.update(binary);
    for name in [
        "claude-manifest.json",
        "codex-marketplace.json",
        "codex-plugin.json",
        "codex-legacy-plugin.json",
        "meeting-summary/SKILL.md",
        "icon.png",
    ] {
        let bytes = bounded_read(
            &resources.join("agent/integrations").join(name),
            4 * 1024 * 1024,
        )
        .map_err(|_| "연결 패키지를 확인하지 못했습니다.")?;
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn load_registration(directory: &Path, root: &Path, client: &str) -> Result<Value, String> {
    let state = read_json(&directory.join("state.json"))?;
    let token = state["token"].as_str().unwrap_or("");
    if state["schemaVersion"] != 1
        || state["client"] != client
        || state["dataDir"].as_str() != root.to_str()
        || !valid_token(token)
    {
        return Err("연결 설정이 일치하지 않습니다.".into());
    }
    Ok(state)
}

fn valid_token(token: &str) -> bool {
    token.len() == 36
        && token.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn live_session(directory: &Path, client: &str, token: &str, generation: &str) -> bool {
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .any(|path| {
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                return false;
            }
            let Ok(value) = read_json(&path) else {
                return false;
            };
            if value["schemaVersion"] != 1
                || value["client"] != client
                || value["token"] != token
                || value["generation"] != generation
            {
                return false;
            }
            let lock_path = path.with_extension("lock");
            if fs::symlink_metadata(&lock_path).map_or(true, |metadata| {
                !metadata.is_file() || metadata.file_type().is_symlink()
            }) {
                return false;
            }
            let Ok(file) = OpenOptions::new().read(true).write(true).open(lock_path) else {
                return false;
            };
            match file.try_lock_exclusive() {
                Ok(()) => {
                    let _ = FileExt::unlock(&file);
                    false
                }
                Err(error) => error.raw_os_error() == fs2::lock_contended_error().raw_os_error(),
            }
        })
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = bounded_read(path, 128 * 1024).map_err(|_| "연결 파일을 읽지 못했습니다.")?;
    serde_json::from_slice(&bytes).map_err(|_| "연결 파일 형식을 확인하지 못했습니다.".into())
}

fn bounded_read(path: &Path, maximum: usize) -> std::io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn immutable_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if path.exists() {
        if bounded_read(path, 64 * 1024 * 1024)? != bytes {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        return Ok(());
    }
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(())
}

fn stored_zip(files: &[(&str, &[u8], u32)]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut directory = Vec::new();
    for (name, content, mode) in files {
        let size = u32::try_from(content.len()).map_err(|_| "연결 파일이 너무 큽니다.")?;
        let offset = u32::try_from(bytes.len()).map_err(|_| "연결 파일이 너무 큽니다.")?;
        let name_len = u16::try_from(name.len()).map_err(|_| "연결 파일 이름이 너무 깁니다.")?;
        let checksum = crc32(content);
        bytes.extend(0x04034b50_u32.to_le_bytes());
        bytes.extend(20_u16.to_le_bytes());
        bytes.extend(0x800_u16.to_le_bytes());
        bytes.extend([0_u8; 4]);
        bytes.extend(0x0021_u16.to_le_bytes());
        bytes.extend(checksum.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(name_len.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(*content);
        directory.extend(0x02014b50_u32.to_le_bytes());
        directory.extend(0x0314_u16.to_le_bytes());
        directory.extend(20_u16.to_le_bytes());
        directory.extend(0x800_u16.to_le_bytes());
        directory.extend([0_u8; 4]);
        directory.extend(0x0021_u16.to_le_bytes());
        directory.extend(checksum.to_le_bytes());
        directory.extend(size.to_le_bytes());
        directory.extend(size.to_le_bytes());
        directory.extend(name_len.to_le_bytes());
        directory.extend([0_u8; 8]);
        directory.extend(((*mode | 0o100000) << 16).to_le_bytes());
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let offset = bytes.len() as u32;
    let size = directory.len() as u32;
    bytes.extend(directory);
    bytes.extend(0x06054b50_u32.to_le_bytes());
    bytes.extend([0_u8; 4]);
    bytes.extend((files.len() as u16).to_le_bytes());
    bytes.extend((files.len() as u16).to_le_bytes());
    bytes.extend(size.to_le_bytes());
    bytes.extend(offset.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    Ok(bytes)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut value = !0_u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn quote(path: &Path, powershell: bool) -> String {
    let path = path.to_string_lossy();
    format!(
        "'{}'",
        path.replace('\'', if powershell { "''" } else { "'\\''" })
    )
}

pub fn resource_agent_path(resources: &Path) -> PathBuf {
    resources.join("agent").join(executable_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(path: &Path) -> PathBuf {
        let resources = path.join("resources");
        let integrations = resources.join("agent/integrations");
        fs::create_dir_all(integrations.join("meeting-summary")).unwrap();
        fs::write(resource_agent_path(&resources), b"synthetic executable").unwrap();
        fs::write(integrations.join("icon.png"), b"icon").unwrap();
        fs::write(
            integrations.join("meeting-summary/SKILL.md"),
            b"summary rules",
        )
        .unwrap();
        fs::write(
            integrations.join("claude-manifest.json"),
            json!({"server":{"mcp_config":{"command":"${__dirname}/server/nero"}}}).to_string(),
        )
        .unwrap();
        fs::write(
            integrations.join("codex-marketplace.json"),
            json!({"plugins":[{"source":{"source":"local","path":"./plugins/nero"}}]}).to_string(),
        )
        .unwrap();
        for name in ["codex-plugin.json", "codex-legacy-plugin.json"] {
            fs::write(integrations.join(name), "{}").unwrap();
        }
        resources
    }

    #[test]
    fn missing_binary_does_not_offer_config_or_create_directories() {
        let temp = tempfile::tempdir().unwrap();
        let resources = temp.path().join("missing resources");
        let info = connection_info(
            &resources,
            &temp.path().join("missing data"),
            &temp.path().join("connections"),
        );
        assert!(!info.available);
        assert!(info.config.is_null());
        assert!(info.cli_example.is_empty());
        assert!(!resources.exists());
        assert!(info
            .clients
            .iter()
            .all(|client| client.state == "unavailable"));
    }

    #[test]
    fn prepares_paths_without_shell_or_writing_the_library_and_reuses_registration() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("한글 user's library");
        let connections = temp.path().join("한글 user's connections");
        let action = prepare_connection(&resources, &root, &connections, "codex").unwrap();
        assert!(action
            .target
            .starts_with("codex://plugins/nero?marketplacePath="));
        assert!(
            action.target.contains("%20")
                && action.target.contains("%27")
                && action.target.contains("%ED")
        );
        assert!(!root.exists());
        let registration = fs::read(connections.join("codex/state.json")).unwrap();
        let state: Value = serde_json::from_slice(&registration).unwrap();
        let plugin = connections
            .join("codex/releases")
            .join(state["generation"].as_str().unwrap())
            .join("plugins/nero");
        let portable = read_json(&plugin.join("mcp.json")).unwrap();
        assert_eq!(
            portable["mcpServers"]["nero"]["command"],
            format!("./server/{}", executable_name())
        );
        let installed = plugin.join("server").join(executable_name());
        assert_eq!(
            fs::read(&installed).unwrap(),
            fs::read(resource_agent_path(&resources)).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        let second = prepare_connection(&resources, &root, &connections, "codex").unwrap();
        assert_eq!(action.target, second.target);
        assert_eq!(
            registration,
            fs::read(connections.join("codex/state.json")).unwrap()
        );
        let info = connection_info(&resources, &root, &connections);
        assert_eq!(info.clients[1].state, "waiting");
        assert_eq!(
            info.config["mcpServers"]["nero"]["args"],
            json!(["--data-dir", root, "mcp"])
        );
    }

    #[test]
    fn rejects_unknown_clients_before_creating_anything() {
        let temp = tempfile::tempdir().unwrap();
        let connections = temp.path().join("connections");
        assert!(
            prepare_connection(temp.path(), temp.path(), &connections, "https://other").is_err()
        );
        assert!(!connections.exists());
    }

    #[test]
    fn only_a_locked_matching_initialized_receipt_is_connected() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing data");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "claude").unwrap();
        let registration = load_registration(&connections.join("claude"), &root, "claude").unwrap();
        let path = connections.join("claude/sessions/session.json");
        let mut receipt = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        receipt
            .write_all(
                json!({"schemaVersion":1,"client":"claude","token":registration["token"],"generation":registration["generation"]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[0].state,
            "waiting"
        );
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(connections.join("claude/sessions/session.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[0].state,
            "connected"
        );
        drop(lock);
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[0].state,
            "waiting"
        );
        assert!(!root.exists());
    }

    #[test]
    fn zip_uses_standard_crc_and_preserves_executable_mode() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        let zip = stored_zip(&[("server/nero", b"native", 0o755)]).unwrap();
        assert_eq!(&zip[..4], &0x04034b50_u32.to_le_bytes());
        let central = zip
            .windows(4)
            .position(|bytes| bytes == 0x02014b50_u32.to_le_bytes())
            .unwrap();
        let permissions = u32::from_le_bytes(zip[central + 38..central + 42].try_into().unwrap());
        assert_eq!(permissions >> 16, 0o100755);
    }
    #[test]
    fn old_live_generation_requires_update_and_reuses_the_registration_token() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let before = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        let session = connections.join("codex/sessions/old.json");
        fs::write(&session,json!({"schemaVersion":1,"client":"codex","token":before["token"],"generation":before["generation"]}).to_string()).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(session.with_extension("lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "connected"
        );
        fs::write(
            resource_agent_path(&resources),
            b"updated synthetic executable",
        )
        .unwrap();
        let info = connection_info(&resources, &root, &connections);
        assert_eq!(info.clients[1].state, "waiting");
        assert_eq!(
            info.clients[1].detail.as_deref(),
            Some("연결을 업데이트해 주세요.")
        );
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let after = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        assert_eq!(before["token"], after["token"]);
        assert_ne!(before["generation"], after["generation"]);
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "waiting"
        );
        assert!(!root.exists());
    }

    #[test]
    fn stale_receipts_cannot_hide_an_older_still_live_session() {
        let temp = tempfile::tempdir().unwrap();
        let token = "11111111-1111-4111-8111-111111111111";
        let generation = "a".repeat(64);
        let session = temp.path().join("00000000000000000000000000000000.json");
        fs::write(
            &session,
            json!({"schemaVersion":1,"client":"codex","token":token,"generation":generation})
                .to_string(),
        )
        .unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(session.with_extension("lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        for i in 1..=140 {
            fs::write(temp.path().join(format!("{i:032}.json")), "{}").unwrap();
        }
        assert!(live_session(temp.path(), "codex", token, &generation));
    }
    #[test]
    fn template_only_package_updates_change_generation_with_the_same_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let before = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        let binary = fs::read(resource_agent_path(&resources)).unwrap();
        fs::write(
            resources.join("agent/integrations/meeting-summary/SKILL.md"),
            b"updated meeting rules",
        )
        .unwrap();
        let info = connection_info(&resources, &root, &connections);
        assert_eq!(
            info.clients[1].detail.as_deref(),
            Some("연결을 업데이트해 주세요.")
        );
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let after = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        assert_eq!(fs::read(resource_agent_path(&resources)).unwrap(), binary);
        assert_ne!(before["generation"], after["generation"]);
        assert_eq!(before["token"], after["token"]);
    }
}
