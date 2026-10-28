use fs2::FileExt;
use serde::{Deserialize, Serialize};
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

pub struct ConnectionObservationError {
    client: String,
    detail: String,
}

impl ConnectionInfo {
    pub fn with_observation_errors(mut self, errors: Vec<ConnectionObservationError>) -> Self {
        for error in errors {
            if let Some(client) = self
                .clients
                .iter_mut()
                .find(|client| client.id == error.client)
            {
                client.state = "error".into();
                client.detail = Some(error.detail);
            }
        }
        self
    }
}

pub struct ConnectionAction {
    pub kind: String,
    pub target: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectionObservation {
    schema_version: u32,
    client: String,
    token_hash: String,
    verified_generation: Option<String>,
    pending_generation: Option<String>,
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
            let current_live = prepared.as_ref().is_some_and(|registration| {
                live_session(
                    &connections.join(id).join("sessions"),
                    id,
                    registration["token"].as_str().unwrap_or(""),
                    current_generation.as_deref().unwrap_or(""),
                )
            });
            let observation = prepared.as_ref().and_then(|registration| {
                load_observation(&connections.join(id), id, registration)
                    .ok()
                    .flatten()
            });
            let update_required = !current_live
                && prepared
                    .as_ref()
                    .zip(current_generation.as_deref())
                    .is_some_and(|(registration, generation)| {
                        registration["generation"].as_str() != Some(generation)
                            || observation.as_ref().is_some_and(|value| {
                                value.pending_generation.as_deref() == Some(generation)
                                    && value.verified_generation.as_deref() != Some(generation)
                            })
                    });
            let state = if !available || !templates.is_dir() || current_generation.is_none() {
                "unavailable"
            } else if current_live {
                "connected"
            } else if update_required {
                "update_required"
            } else if prepared.is_some() {
                "waiting"
            } else {
                "not_connected"
            };
            ClientInfo {
                id: (*id).into(),
                name: (*name).into(),
                state: state.into(),
                detail: None,
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

pub fn observe_connection_state(
    resources: &Path,
    root: &Path,
    connections: &Path,
) -> Vec<ConnectionObservationError> {
    let Ok(binary) = bounded_read(&resource_agent_path(resources), 64 * 1024 * 1024) else {
        return Vec::new();
    };
    let Ok(generation) = package_generation(resources, &binary) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    for (client, _) in CLIENTS {
        let directory = connections.join(client);
        if !plain_directory(&directory) || load_registration(&directory, root, client).is_err() {
            continue;
        }
        if let Err(detail) = observe_registered_connection(&directory, root, client, &generation) {
            errors.push(ConnectionObservationError {
                client: client.into(),
                detail,
            });
        }
    }
    errors
}

fn observe_registered_connection(
    directory: &Path,
    root: &Path,
    client: &str,
    generation: &str,
) -> Result<(), String> {
    let _lock = lock_preparation(directory)?;
    let registration = load_registration(directory, root, client)?;
    let previous = load_observation(directory, client, &registration)?;
    let mut observation = previous
        .clone()
        .unwrap_or_else(|| new_observation(client, &registration));
    let active = live_generations(
        &directory.join("sessions"),
        client,
        registration["token"].as_str().unwrap_or(""),
    );
    if active.iter().any(|value| value == generation) {
        observation.verified_generation = Some(generation.into());
        observation.pending_generation = None;
    } else if observation.verified_generation.as_deref() != Some(generation)
        && (registration["generation"].as_str() != Some(generation)
            || observation.pending_generation.is_some()
            || observation.verified_generation.is_some()
            || active.iter().any(|value| {
                value != generation
                    && historical_release(directory, root, client, &registration, value)
            }))
    {
        observation.pending_generation = Some(generation.into());
    }
    save_observation(directory, previous.as_ref(), &observation)?;
    Ok(())
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
    let _preparation_lock = lock_preparation(&client_directory)?;
    let codex_marketplace = if client == "codex" {
        Some(
            codex_marketplace_path(&client_directory)
                .map_err(|_| "Codex 설치 경로를 준비하지 못했습니다.")?,
        )
    } else {
        None
    };
    let state_path = client_directory.join("state.json");
    let existing_registration = fs::symlink_metadata(&state_path).is_ok();
    let mut registration = if existing_registration {
        load_registration(&client_directory, root, client)
            .map_err(|_| "기존 연결 경로를 확인하지 못했습니다.")?
    } else {
        let value = json!({"schemaVersion":1,"client":client,"token":uuid::Uuid::new_v4().to_string(),"dataDir":root,"generation":generation});
        atomic_write(&state_path, value.to_string().as_bytes())
            .map_err(|_| "연결 설정을 저장하지 못했습니다.")?;
        value
    };
    let previous_observation = load_observation(&client_directory, client, &registration)?;
    let mut observation = previous_observation
        .clone()
        .unwrap_or_else(|| new_observation(client, &registration));
    if existing_registration
        && observation.verified_generation.as_deref() != Some(generation.as_str())
        && (registration["generation"].as_str() != Some(generation.as_str())
            || observation.pending_generation.is_some())
    {
        observation.pending_generation = Some(generation.clone());
    }
    save_observation(
        &client_directory,
        previous_observation.as_ref(),
        &observation,
    )?;
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
    *marketplace
        .pointer_mut("/plugins/0/source/path")
        .ok_or("Codex 설치 정보를 확인하지 못했습니다.")? =
        json!(format!("./releases/{generation}/plugins/nero"));
    let marketplace_path = codex_marketplace.ok_or("Codex 설치 경로를 확인하지 못했습니다.")?;
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

fn codex_marketplace_path(directory: &Path) -> std::io::Result<PathBuf> {
    let agents = directory.join(".agents");
    private_directory(&agents)?;
    let catalogs = agents.join("plugins");
    private_directory(&catalogs)?;
    let path = catalogs.join("marketplace.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            Err(std::io::ErrorKind::InvalidInput.into())
        }
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error),
    }
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

fn valid_generation(generation: &str) -> bool {
    generation.len() == 64 && generation.bytes().all(|value| value.is_ascii_hexdigit())
}

fn new_observation(client: &str, registration: &Value) -> ConnectionObservation {
    ConnectionObservation {
        schema_version: 1,
        client: client.into(),
        token_hash: format!(
            "{:x}",
            Sha256::digest(registration["token"].as_str().unwrap_or("").as_bytes())
        ),
        verified_generation: None,
        pending_generation: None,
    }
}

fn load_observation(
    directory: &Path,
    client: &str,
    registration: &Value,
) -> Result<Option<ConnectionObservation>, String> {
    let path = directory.join("observation.json");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("연결 확인 기록을 읽지 못했습니다.".into()),
        Ok(_) => {}
    }
    let bytes = bounded_read(&path, 4096).map_err(|_| "연결 확인 기록을 읽지 못했습니다.")?;
    let observation: ConnectionObservation =
        serde_json::from_slice(&bytes).map_err(|_| "연결 확인 기록을 확인하지 못했습니다.")?;
    let expected = new_observation(client, registration);
    if observation.schema_version != 1
        || observation.client != expected.client
        || observation.token_hash != expected.token_hash
        || [
            observation.verified_generation.as_deref(),
            observation.pending_generation.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !valid_generation(value))
    {
        return Err("연결 확인 기록이 일치하지 않습니다.".into());
    }
    Ok(Some(observation))
}

fn save_observation(
    directory: &Path,
    previous: Option<&ConnectionObservation>,
    observation: &ConnectionObservation,
) -> Result<(), String> {
    if previous == Some(observation) {
        return Ok(());
    }
    atomic_write(
        &directory.join("observation.json"),
        &serde_json::to_vec(observation).map_err(|_| "연결 확인 기록을 만들지 못했습니다.")?,
    )
    .map_err(|_| "연결 확인 기록을 저장하지 못했습니다.".into())
}

fn plain_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn lock_preparation(directory: &Path) -> Result<File, String> {
    let path = directory.join("prepare.lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("연결 잠금 경로를 확인하지 못했습니다.".into());
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("연결 잠금 경로를 확인하지 못했습니다.".into());
        }
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| "연결을 준비하지 못했습니다.")?;
    file.try_lock_exclusive()
        .map_err(|_| "연결을 준비 중입니다. 잠시 후 다시 시도해 주세요.")?;
    Ok(file)
}

fn live_session(directory: &Path, client: &str, token: &str, generation: &str) -> bool {
    live_generations(directory, client, token)
        .iter()
        .any(|value| value == generation)
}

fn live_generations(directory: &Path, client: &str, token: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut generations = Vec::new();
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Some(value) = initialized_session(&path, client, token) else {
            continue;
        };
        let generation = value["generation"].as_str().unwrap_or("");
        if !generations.iter().any(|value| value == generation) {
            generations.push(generation.to_owned());
        }
    }
    generations
}

fn initialized_session(path: &Path, client: &str, token: &str) -> Option<Value> {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return None;
    }
    let value = read_json(path).ok()?;
    if value["schemaVersion"] != 1
        || value["client"] != client
        || value["token"] != token
        || !valid_generation(value["generation"].as_str().unwrap_or(""))
    {
        return None;
    }
    let lock_path = path.with_extension("lock");
    if fs::symlink_metadata(&lock_path).map_or(true, |metadata| {
        !metadata.is_file() || metadata.file_type().is_symlink()
    }) {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .ok()?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            None
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Some(value)
        }
        Err(_) => None,
    }
}

fn historical_release(
    directory: &Path,
    root: &Path,
    client: &str,
    registration: &Value,
    generation: &str,
) -> bool {
    if !valid_generation(generation) {
        return false;
    }
    let releases = directory.join("releases");
    let release = releases.join(generation);
    if !plain_directory(&releases) || !plain_directory(&release) {
        return false;
    }
    if client == "claude" {
        return bounded_read(&release.join("Nero.mcpb"), 64 * 1024 * 1024)
            .is_ok_and(|bytes| bytes.starts_with(&0x04034b50_u32.to_le_bytes()));
    }
    let plugins = release.join("plugins");
    let plugin = plugins.join("nero");
    let server = plugin.join("server");
    if ![plugins, plugin.clone(), server.clone()]
        .iter()
        .all(|path| plain_directory(path))
    {
        return false;
    }
    let executable = server.join(executable_name());
    if !bounded_read(&executable, 64 * 1024 * 1024).is_ok_and(|bytes| !bytes.is_empty()) {
        return false;
    }
    let Ok(config) = read_json(&plugin.join(".mcp.json")) else {
        return false;
    };
    let entry = &config["mcpServers"]["nero"];
    let environment = &entry["env"];
    entry["command"] == json!(executable)
        && entry["args"] == json!(["--data-dir", root, "mcp"])
        && environment["NERO_CONNECTION_CLIENT"] == client
        && environment["NERO_CONNECTION_TOKEN"] == registration["token"]
        && environment["NERO_CONNECTION_GENERATION"] == generation
        && environment["NERO_CONNECTION_DIR"] == json!(directory.join("sessions"))
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

    fn initialized_receipt(
        directory: &Path,
        registration: &Value,
        generation: &str,
        name: &str,
    ) -> File {
        let path = directory.join("sessions").join(format!("{name}.json"));
        fs::write(
            &path,
            json!({"schemaVersion":1,"client":registration["client"],"token":registration["token"],"generation":generation})
                .to_string(),
        )
        .unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.with_extension("lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        lock
    }

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
    fn observing_without_a_registration_does_not_create_a_directory() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("missing connections");

        assert!(observe_connection_state(&resources, &root, &connections).is_empty());

        assert!(!connections.exists());
        assert!(!root.exists());
    }

    #[test]
    fn prepared_update_remains_required_until_an_observed_current_session() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let before = load_registration(&client, &root, "codex").unwrap();
        let initial_observation = read_json(&client.join("observation.json")).unwrap();
        assert!(initial_observation["verifiedGeneration"].is_null());
        assert!(initial_observation["pendingGeneration"].is_null());
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "waiting"
        );
        let old_lock = initialized_receipt(
            &client,
            &before,
            before["generation"].as_str().unwrap(),
            "previous",
        );
        fs::write(resource_agent_path(&resources), b"new synthetic executable").unwrap();

        prepare_connection(&resources, &root, &connections, "codex").unwrap();

        let current = load_registration(&client, &root, "codex").unwrap();
        let generation = current["generation"].as_str().unwrap();
        let registration_bytes = fs::read(client.join("state.json")).unwrap();
        let pending_bytes = fs::read(client.join("observation.json")).unwrap();
        let pending = read_json(&client.join("observation.json")).unwrap();
        assert_eq!(before["token"], current["token"]);
        assert_eq!(pending["pendingGeneration"], generation);
        assert!(pending["verifiedGeneration"].is_null());
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "update_required"
        );
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        assert!(observe_connection_state(&resources, &root, &connections).is_empty());
        assert_eq!(
            fs::read(client.join("observation.json")).unwrap(),
            pending_bytes
        );
        let current_lock = initialized_receipt(&client, &current, generation, "current");
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "connected"
        );
        assert_eq!(
            fs::read(client.join("observation.json")).unwrap(),
            pending_bytes
        );

        assert!(observe_connection_state(&resources, &root, &connections).is_empty());

        let confirmed = read_json(&client.join("observation.json")).unwrap();
        assert_eq!(confirmed["verifiedGeneration"], generation);
        assert!(confirmed["pendingGeneration"].is_null());
        let confirmed_bytes = fs::read(client.join("observation.json")).unwrap();
        drop(current_lock);
        assert!(observe_connection_state(&resources, &root, &connections).is_empty());
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "waiting"
        );
        assert_eq!(
            fs::read(client.join("observation.json")).unwrap(),
            confirmed_bytes
        );
        drop(old_lock);
        assert!(observe_connection_state(&resources, &root, &connections).is_empty());
        assert_eq!(
            fs::read(client.join("observation.json")).unwrap(),
            confirmed_bytes
        );
        assert_eq!(
            fs::read(client.join("state.json")).unwrap(),
            registration_bytes
        );
        assert!(!root.exists());
    }

    #[test]
    fn legacy_prepared_registration_recovers_pending_only_from_known_live_history() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let old = load_registration(&client, &root, "codex").unwrap();
        let _old_lock =
            initialized_receipt(&client, &old, old["generation"].as_str().unwrap(), "legacy");
        fs::write(resource_agent_path(&resources), b"new synthetic executable").unwrap();
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        fs::remove_file(client.join("observation.json")).unwrap();
        let current = load_registration(&client, &root, "codex").unwrap();
        let registration_bytes = fs::read(client.join("state.json")).unwrap();
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "waiting"
        );
        assert!(!client.join("observation.json").exists());

        assert!(observe_connection_state(&resources, &root, &connections).is_empty());

        let observation = read_json(&client.join("observation.json")).unwrap();
        assert_eq!(observation["pendingGeneration"], current["generation"]);
        assert!(observation["verifiedGeneration"].is_null());
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "update_required"
        );
        assert_eq!(
            fs::read(client.join("state.json")).unwrap(),
            registration_bytes
        );
        assert!(!root.exists());
    }

    #[test]
    fn unknown_old_receipt_does_not_invent_an_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let registration = load_registration(&client, &root, "codex").unwrap();
        let _unknown_lock = initialized_receipt(&client, &registration, &"a".repeat(64), "unknown");
        fs::remove_file(client.join("observation.json")).unwrap();

        assert!(observe_connection_state(&resources, &root, &connections).is_empty());

        let observation = read_json(&client.join("observation.json")).unwrap();
        assert!(observation["pendingGeneration"].is_null());
        assert!(observation["verifiedGeneration"].is_null());
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "waiting"
        );
    }

    #[test]
    fn invalid_observations_are_preserved_and_do_not_change_registration() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let original_registration = fs::read(client.join("state.json")).unwrap();
        let valid = read_json(&client.join("observation.json")).unwrap();
        let mut wrong_schema = valid.clone();
        wrong_schema["schemaVersion"] = json!(2);
        let mut wrong_identity = valid.clone();
        wrong_identity["tokenHash"] = json!("b".repeat(64));
        let mut wrong_generation = valid.clone();
        wrong_generation["pendingGeneration"] = json!("../outside");
        for bytes in [
            b"not json".to_vec(),
            wrong_schema.to_string().into_bytes(),
            wrong_identity.to_string().into_bytes(),
            wrong_generation.to_string().into_bytes(),
        ] {
            fs::write(client.join("observation.json"), &bytes).unwrap();
            assert_eq!(
                observe_connection_state(&resources, &root, &connections).len(),
                1
            );
            assert!(prepare_connection(&resources, &root, &connections, "codex").is_err());
            assert_eq!(fs::read(client.join("observation.json")).unwrap(), bytes);
            assert_eq!(
                fs::read(client.join("state.json")).unwrap(),
                original_registration
            );
        }
        assert!(!root.exists());
    }

    #[test]
    fn observation_waits_for_preparation_lock_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let original_registration = fs::read(client.join("state.json")).unwrap();
        let original_observation = fs::read(client.join("observation.json")).unwrap();
        let _held = lock_preparation(&client).unwrap();

        assert_eq!(
            observe_connection_state(&resources, &root, &connections).len(),
            1
        );
        assert!(prepare_connection(&resources, &root, &connections, "codex").is_err());

        assert_eq!(
            fs::read(client.join("state.json")).unwrap(),
            original_registration
        );
        assert_eq!(
            fs::read(client.join("observation.json")).unwrap(),
            original_observation
        );
        assert!(!root.exists());
    }

    #[test]
    fn one_client_observation_failure_does_not_block_the_other_client() {
        for failure in ["invalid record", "held lock"] {
            let temp = tempfile::tempdir().unwrap();
            let resources = resources(temp.path());
            let root = temp.path().join("missing library");
            let connections = temp.path().join("connections");
            prepare_connection(&resources, &root, &connections, "claude").unwrap();
            prepare_connection(&resources, &root, &connections, "codex").unwrap();
            let claude = connections.join("claude");
            let codex = connections.join("codex");
            let claude_state = fs::read(claude.join("state.json")).unwrap();
            let codex_state = fs::read(codex.join("state.json")).unwrap();
            let registration = load_registration(&codex, &root, "codex").unwrap();
            let generation = registration["generation"].as_str().unwrap();
            let _current = initialized_receipt(&codex, &registration, generation, "healthy");
            if failure == "invalid record" {
                fs::write(claude.join("observation.json"), b"preserve invalid record").unwrap();
            }
            let original_observation = fs::read(claude.join("observation.json")).unwrap();
            let _held = (failure == "held lock").then(|| lock_preparation(&claude).unwrap());

            let errors = observe_connection_state(&resources, &root, &connections);

            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].client, "claude");
            assert!(!errors[0].detail.contains(root.to_str().unwrap()));
            assert!(!errors[0]
                .detail
                .contains(registration["token"].as_str().unwrap()));
            let info =
                connection_info(&resources, &root, &connections).with_observation_errors(errors);
            assert_eq!(info.clients[0].state, "error");
            assert!(info.clients[0].detail.is_some());
            assert_eq!(info.clients[1].state, "connected");
            assert!(info.clients[1].detail.is_none());
            assert_eq!(
                read_json(&codex.join("observation.json")).unwrap()["verifiedGeneration"],
                generation
            );
            prepare_connection(&resources, &root, &connections, "codex").unwrap();
            assert_eq!(fs::read(claude.join("state.json")).unwrap(), claude_state);
            assert_eq!(fs::read(codex.join("state.json")).unwrap(), codex_state);
            assert_eq!(
                fs::read(claude.join("observation.json")).unwrap(),
                original_observation
            );
            assert!(!root.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn linked_observation_and_preparation_lock_do_not_touch_external_files() {
        use std::os::unix::fs::symlink;

        for name in ["observation.json", "prepare.lock"] {
            let temp = tempfile::tempdir().unwrap();
            let resources = resources(temp.path());
            let root = temp.path().join("missing library");
            let connections = temp.path().join("connections");
            prepare_connection(&resources, &root, &connections, "codex").unwrap();
            let client = connections.join("codex");
            let original_registration = fs::read(client.join("state.json")).unwrap();
            let external = temp.path().join("outside.json");
            fs::write(&external, b"preserve external file").unwrap();
            fs::remove_file(client.join(name)).unwrap();
            symlink(&external, client.join(name)).unwrap();

            assert_eq!(
                observe_connection_state(&resources, &root, &connections).len(),
                1
            );
            assert!(prepare_connection(&resources, &root, &connections, "codex").is_err());

            assert_eq!(fs::read(&external).unwrap(), b"preserve external file");
            assert!(fs::symlink_metadata(client.join(name))
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                fs::read(client.join("state.json")).unwrap(),
                original_registration
            );
            assert!(!root.exists());
        }
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
        let marketplace_path = connections.join("codex/.agents/plugins/marketplace.json");
        assert_eq!(
            action.target,
            format!(
                "codex://plugins/nero?marketplacePath={}",
                percent_encode(&marketplace_path.to_string_lossy())
            )
        );
        let marketplace = read_json(&marketplace_path).unwrap();
        assert_eq!(
            marketplace["plugins"][0]["source"]["path"],
            format!(
                "./releases/{}/plugins/nero",
                state["generation"].as_str().unwrap()
            )
        );
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
    fn codex_catalog_stays_at_one_path_and_preserves_previous_generations() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        let first = prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let client = connections.join("codex");
        let before = load_registration(&client, &root, "codex").unwrap();
        let previous = client
            .join("releases")
            .join(before["generation"].as_str().unwrap());
        let previous_binary = previous.join("plugins/nero/server").join(executable_name());
        let original_binary = fs::read(&previous_binary).unwrap();
        let legacy_directory = previous.join(".agents/plugins");
        fs::create_dir_all(&legacy_directory).unwrap();
        let legacy_catalog = legacy_directory.join("marketplace.json");
        let original_catalog =
            json!({"plugins":[{"source":{"source":"local","path":"./plugins/nero"}}]}).to_string();
        fs::write(&legacy_catalog, &original_catalog).unwrap();
        let stable_path = client.join(".agents/plugins/marketplace.json");
        let first_catalog = read_json(&stable_path).unwrap();
        fs::write(
            resource_agent_path(&resources),
            b"updated synthetic executable",
        )
        .unwrap();

        let second = prepare_connection(&resources, &root, &connections, "codex").unwrap();

        let after = load_registration(&client, &root, "codex").unwrap();
        let second_catalog = read_json(&stable_path).unwrap();
        assert_eq!(first.target, second.target);
        assert_eq!(before["token"], after["token"]);
        assert_ne!(before["generation"], after["generation"]);
        let mut expected_catalog = first_catalog;
        expected_catalog["plugins"][0]["source"]["path"] = json!(format!(
            "./releases/{}/plugins/nero",
            after["generation"].as_str().unwrap()
        ));
        assert_eq!(second_catalog, expected_catalog);
        assert_eq!(fs::read(&previous_binary).unwrap(), original_binary);
        assert_eq!(
            fs::read(&legacy_catalog).unwrap(),
            original_catalog.as_bytes()
        );
        let current_plugin = client.join(
            second_catalog["plugins"][0]["source"]["path"]
                .as_str()
                .unwrap(),
        );
        assert_eq!(
            fs::read(current_plugin.join("server").join(executable_name())).unwrap(),
            b"updated synthetic executable"
        );
        assert!(!root.exists());
    }

    #[test]
    fn explicit_preparation_adopts_a_legacy_catalog_without_replacing_its_release() {
        let temp = tempfile::tempdir().unwrap();
        let resources = resources(temp.path());
        let root = temp.path().join("missing library");
        let connections = temp.path().join("connections");
        let client = connections.join("codex");
        let old_generation =
            package_generation(&resources, b"legacy synthetic executable").unwrap();
        let legacy_release = client.join("releases").join(&old_generation);
        let legacy_server = legacy_release.join("plugins/nero/server");
        fs::create_dir_all(&legacy_server).unwrap();
        let legacy_executable = legacy_server.join(executable_name());
        fs::write(&legacy_executable, b"legacy synthetic executable").unwrap();
        let legacy_catalog = legacy_release.join(".agents/plugins/marketplace.json");
        fs::create_dir_all(legacy_catalog.parent().unwrap()).unwrap();
        let legacy_bytes =
            json!({"plugins":[{"source":{"source":"local","path":"./plugins/nero"}}]}).to_string();
        fs::write(&legacy_catalog, &legacy_bytes).unwrap();
        let old_registration = json!({"schemaVersion":1,"client":"codex","token":"11111111-1111-4111-8111-111111111111","dataDir":root,"generation":old_generation});
        fs::write(client.join("state.json"), old_registration.to_string()).unwrap();
        let stable_path = client.join(".agents/plugins/marketplace.json");
        assert!(!stable_path.exists());

        let action = prepare_connection(&resources, &root, &connections, "codex").unwrap();

        let current = load_registration(&client, &root, "codex").unwrap();
        assert_eq!(old_registration["token"], current["token"]);
        assert_ne!(old_registration["generation"], current["generation"]);
        assert_eq!(
            action.target,
            format!(
                "codex://plugins/nero?marketplacePath={}",
                percent_encode(&stable_path.to_string_lossy())
            )
        );
        assert_eq!(
            read_json(&stable_path).unwrap()["plugins"][0]["source"]["path"],
            format!(
                "./releases/{}/plugins/nero",
                current["generation"].as_str().unwrap()
            )
        );
        assert_eq!(fs::read(&legacy_catalog).unwrap(), legacy_bytes.as_bytes());
        assert_eq!(
            fs::read(&legacy_executable).unwrap(),
            b"legacy synthetic executable"
        );
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "update_required"
        );
        assert!(!root.exists());
    }

    #[test]
    fn codex_catalog_rejects_non_file_destinations_before_changing_registration() {
        for invalid in [
            ".agents",
            ".agents/plugins",
            ".agents/plugins/marketplace.json",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let resources = resources(temp.path());
            let root = temp.path().join("missing library");
            let connections = temp.path().join("connections");
            let client = connections.join("codex");
            let path = client.join(invalid);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if invalid.ends_with(".json") {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, b"preserve this file").unwrap();
            }

            assert!(prepare_connection(&resources, &root, &connections, "codex").is_err());

            assert!(!client.join("state.json").exists());
            assert!(!client.join("releases").exists());
            if !invalid.ends_with(".json") {
                assert_eq!(fs::read(&path).unwrap(), b"preserve this file");
            }
            assert!(!root.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_catalog_rejects_linked_components_and_keeps_external_files_untouched() {
        use std::os::unix::fs::symlink;

        for invalid in [
            ".agents",
            ".agents/plugins",
            ".agents/plugins/marketplace.json",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let resources = resources(temp.path());
            let root = temp.path().join("missing library");
            let connections = temp.path().join("connections");
            let client = connections.join("codex");
            let outside = temp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            let sentinel = outside.join("keep.json");
            fs::write(&sentinel, b"preserve external file").unwrap();
            let path = client.join(invalid);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(
                if invalid.ends_with(".json") {
                    &sentinel
                } else {
                    &outside
                },
                &path,
            )
            .unwrap();

            assert!(prepare_connection(&resources, &root, &connections, "codex").is_err());

            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(&sentinel).unwrap(), b"preserve external file");
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
            assert!(!client.join("state.json").exists());
            assert!(!client.join("releases").exists());
            assert!(!root.exists());
        }
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
        assert_eq!(info.clients[1].state, "update_required");
        assert!(info.clients[1].detail.is_none());
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let after = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        assert_eq!(before["token"], after["token"]);
        assert_ne!(before["generation"], after["generation"]);
        assert_eq!(
            connection_info(&resources, &root, &connections).clients[1].state,
            "update_required"
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
        assert_eq!(info.clients[1].state, "update_required");
        assert!(info.clients[1].detail.is_none());
        prepare_connection(&resources, &root, &connections, "codex").unwrap();
        let after = load_registration(&connections.join("codex"), &root, "codex").unwrap();
        assert_eq!(fs::read(resource_agent_path(&resources)).unwrap(), binary);
        assert_ne!(before["generation"], after["generation"]);
        assert_eq!(before["token"], after["token"]);
    }
}
