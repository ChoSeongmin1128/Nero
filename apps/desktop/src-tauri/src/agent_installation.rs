use super::{bounded_read, executable_name, payload_generation, read_json, ClientInfo};
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) enum HostInstallation {
    Missing,
    Installed {
        directory: PathBuf,
        version: String,
        enabled: bool,
    },
    Unknown(&'static str),
}

#[derive(Clone)]
pub(super) struct HostInstallations {
    pub(super) claude: HostInstallation,
    pub(super) codex: HostInstallation,
}

impl HostInstallations {
    pub(super) fn get(&self, client: &str) -> Option<&HostInstallation> {
        match client {
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            _ => None,
        }
    }
}

struct CachedInstallations {
    roots: (PathBuf, PathBuf, PathBuf, PathBuf),
    observed: Instant,
    installations: HostInstallations,
}

static CACHE: OnceLock<Mutex<Option<CachedInstallations>>> = OnceLock::new();

fn invalidate_cache() {
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *cache = None;
}

pub(super) fn probe(
    home: &Path,
    config: &Path,
    connections: &Path,
    force_refresh: bool,
) -> HostInstallations {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".codex"));
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    probe_cached(
        &mut cache,
        (
            home.to_owned(),
            config.to_owned(),
            connections.to_owned(),
            codex_home.clone(),
        ),
        force_refresh,
        || {
            let claude = claude_installation(&config.join("Claude"));
            let codex = match codex_list(home) {
                Ok(bytes) => codex_installation(&bytes, &codex_home, &connections.join("codex")),
                Err(reason) => HostInstallation::Unknown(reason),
            };
            HostInstallations { claude, codex }
        },
    )
}

fn probe_cached(
    cache: &mut Option<CachedInstallations>,
    roots: (PathBuf, PathBuf, PathBuf, PathBuf),
    force_refresh: bool,
    inspect: impl FnOnce() -> HostInstallations,
) -> HostInstallations {
    if !force_refresh {
        if let Some(cached) = cache.as_ref().filter(|cached| {
            cached.roots == roots && cached.observed.elapsed() < Duration::from_secs(30)
        }) {
            return cached.installations.clone();
        }
    }
    let installations = inspect();
    *cache = Some(CachedInstallations {
        roots,
        observed: Instant::now(),
        installations: installations.clone(),
    });
    installations
}

fn native_executable(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let mut file = fs::File::open(&path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut magic = [0; 4];
    file.read_exact(&mut magic).ok()?;
    if matches!(
        magic,
        [0x7f, b'E', b'L', b'F']
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    ) || magic[..2] == *b"MZ"
    {
        Some(path)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn codex_desktop_executable(bundle: &Path) -> Option<PathBuf> {
    let info = bundle.join("Contents/Info.plist");
    if !info.is_file() {
        return None;
    }
    let mut identifier = Command::new("/usr/bin/plutil");
    identifier
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(info)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let identifier = codex_list_output(identifier).ok()?;
    if std::str::from_utf8(&identifier).ok()?.trim() != "com.openai.codex" {
        return None;
    }
    [
        "Contents/Resources/codex-cli/bin/codex",
        "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
    ]
    .iter()
    .find_map(|relative| native_executable(&bundle.join(relative)))
}

fn codex_command(home: &Path) -> Option<Command> {
    let binary = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(binary))
            .find_map(|path| native_executable(&path))
    }) {
        return Some(Command::new(path));
    }
    #[cfg(target_os = "macos")]
    {
        for path in ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"] {
            if let Some(path) = native_executable(Path::new(path)) {
                return Some(Command::new(path));
            }
        }
        for directory in [PathBuf::from("/Applications"), home.join("Applications")] {
            for name in ["ChatGPT.app", "Codex.app"] {
                if let Some(path) = codex_desktop_executable(&directory.join(name)) {
                    return Some(Command::new(path));
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = home;
    None
}

fn codex_list(home: &Path) -> Result<Vec<u8>, &'static str> {
    let mut command = codex_command(home)
        .ok_or("Codex의 설치 정보를 읽을 수 없습니다. Codex에서 Nero 설치 상태를 확인해 주세요.")?;
    command
        .args(["plugin", "list", "--marketplace", "nero-local", "--json"])
        .current_dir(home)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    codex_list_output(command)
}

pub(super) fn install_codex(home: &Path, connection: &Path) -> Result<bool, String> {
    if !home.is_absolute()
        || !connection.is_absolute()
        || connection
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || !connection.ancestors().all(super::plain_directory)
    {
        return Err("Codex 연결 패키지의 경로를 확인하지 못했습니다.".into());
    }
    let Some(command) = codex_command(home) else {
        return Ok(false);
    };
    let executable = command.get_program().to_owned();
    let _lock = super::lock_preparation(connection)?;
    let result = install_codex_with_runner(connection, |arguments| {
        let mut command = Command::new(&executable);
        command
            .args(arguments)
            .current_dir(home)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        codex_list_output(command).map_err(str::to_owned)
    });
    invalidate_cache();
    result.map(|()| true)
}

fn install_codex_with_runner(
    connection: &Path,
    mut run: impl FnMut(&[OsString]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if !connection.is_absolute() || !plain_tree(connection, &connection.join(".agents/plugins")) {
        return Err("Codex 연결 패키지의 경로를 확인하지 못했습니다.".into());
    }
    let marketplace = read_json(&connection.join(".agents/plugins/marketplace.json"))?;
    let plugins = marketplace["plugins"]
        .as_array()
        .ok_or("Codex 연결 패키지를 확인하지 못했습니다.")?;
    if marketplace["name"] != "nero-local"
        || plugins.len() != 1
        || plugins[0]["name"] != "nero"
        || plugins[0]["source"]["source"] != "local"
    {
        return Err("Codex 연결 패키지를 확인하지 못했습니다.".into());
    }
    let source = plugins[0]["source"]["path"]
        .as_str()
        .and_then(|value| value.strip_prefix("./releases/"))
        .and_then(|value| value.strip_suffix("/plugins/nero"))
        .filter(|value| super::valid_generation(value))
        .ok_or("Codex 연결 패키지를 확인하지 못했습니다.")?;
    if !plain_tree(
        connection,
        &connection
            .join("releases")
            .join(source)
            .join("plugins/nero"),
    ) {
        return Err("Codex 연결 패키지를 확인하지 못했습니다.".into());
    }
    let commands = [
        vec![
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            connection.as_os_str().to_owned(),
            "--json".into(),
        ],
        vec![
            "plugin".into(),
            "add".into(),
            "nero@nero-local".into(),
            "--json".into(),
        ],
    ];
    for (index, arguments) in commands.iter().enumerate() {
        let response = run(arguments).map_err(|_| if index == 0 {
            "Codex 연결 목록의 갱신 결과를 확인하지 못했습니다. 현재 설치 상태를 다시 확인해 주세요.".to_string()
        } else {
            "Codex의 Nero 설치 결과를 확인하지 못했습니다. 현재 설치 상태를 다시 확인해 주세요.".to_string()
        })?;
        if !serde_json::from_slice::<Value>(&response).is_ok_and(|value| value.is_object()) {
            return Err("Codex 연결 변경의 결과를 확인하지 못했습니다. 현재 설치 상태를 다시 확인해 주세요.".into());
        }
    }
    Ok(())
}

fn codex_list_output(mut command: Command) -> Result<Vec<u8>, &'static str> {
    const MAXIMUM: u64 = 2 * 1024 * 1024;
    let mut output = tempfile::tempfile().map_err(|_| "Codex 설치 정보를 읽지 못했습니다.")?;
    command.stdout(Stdio::from(
        output
            .try_clone()
            .map_err(|_| "Codex 설치 정보를 읽지 못했습니다.")?,
    ));
    let mut child = command.spawn().map_err(|_| {
        "Codex의 설치 정보를 읽을 수 없습니다. Codex에서 Nero 설치 상태를 확인해 주세요."
    })?;
    let started = Instant::now();
    loop {
        if !output
            .metadata()
            .is_ok_and(|metadata| metadata.len() <= MAXIMUM)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Codex 설치 정보를 읽지 못했습니다.");
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                return Err("Codex 설치 정보를 확인하지 못했습니다. 다시 확인해 주세요.")
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex 설치 정보를 확인하지 못했습니다. 다시 확인해 주세요.");
            }
            Ok(None) if started.elapsed() >= Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex 설치 확인이 지연되고 있습니다. 잠시 후 다시 확인해 주세요.");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "Codex 설치 정보를 읽지 못했습니다.")?;
    let mut bytes = Vec::new();
    output
        .take(MAXIMUM + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Codex 설치 정보를 읽지 못했습니다.")?;
    if bytes.len() as u64 > MAXIMUM {
        return Err("Codex 설치 정보를 읽지 못했습니다.");
    }
    Ok(bytes)
}

fn codex_installation(bytes: &[u8], home: &Path, connection: &Path) -> HostInstallation {
    let result = (|| {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let installed = value.get("installed")?.as_array()?;
        let mut matches = installed
            .iter()
            .filter(|entry| entry["pluginId"] == "nero@nero-local");
        let Some(entry) = matches.next() else {
            return Some(HostInstallation::Missing);
        };
        if matches.next().is_some()
            || entry["name"] != "nero"
            || entry["marketplaceName"] != "nero-local"
            || entry["installed"] != true
        {
            return None;
        }
        let version = entry["version"]
            .as_str()
            .filter(|value| safe_version(value))?;
        let enabled = entry["enabled"].as_bool()?;
        let source = Path::new(entry.pointer("/source/path")?.as_str()?);
        let relative = source.strip_prefix(connection.join("releases")).ok()?;
        let parts = relative.components().collect::<Vec<_>>();
        if parts.len() != 3
            || !matches!(parts[0], Component::Normal(value) if value.to_str().is_some_and(super::valid_generation))
            || parts[1] != Component::Normal("plugins".as_ref())
            || parts[2] != Component::Normal("nero".as_ref())
        {
            return None;
        }
        let directory = home.join("plugins/cache/nero-local/nero").join(version);
        if !plain_tree(home, &directory) {
            return None;
        }
        let manifest = read_json(&directory.join("plugin.json")).ok()?;
        if manifest["name"] != "nero" || manifest["version"] != version {
            return None;
        }
        let configuration = read_json(&directory.join(".mcp.json")).ok()?;
        if configuration["mcpServers"]["nero"]["env"]["NERO_CONNECTION_GENERATION"]
            != parts[0].as_os_str().to_str()?
        {
            return None;
        }
        Some(HostInstallation::Installed {
            directory,
            version: version.into(),
            enabled,
        })
    })();
    result.unwrap_or(HostInstallation::Unknown(
        "Codex의 설치 정보와 Nero 패키지가 일치하지 않습니다. Codex에서 설치 상태를 확인해 주세요.",
    ))
}

fn claude_installation(root: &Path) -> HostInstallation {
    let path = root.join("extensions-installations.json");
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return HostInstallation::Missing;
    }
    let result = (|| {
        if !super::plain_directory(root) {
            return None;
        }
        let bytes = bounded_read(&path, 2 * 1024 * 1024).ok()?;
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        let extensions = value["extensions"].as_object()?;
        let Some(entry) = extensions.get("local.mcpb.nero.nero") else {
            return Some(HostInstallation::Missing);
        };
        let version = entry["version"]
            .as_str()
            .filter(|value| safe_version(value))?;
        if entry["id"] != "local.mcpb.nero.nero" {
            return None;
        }
        let directory = root.join("Claude Extensions/local.mcpb.nero.nero");
        if !plain_tree(root, &directory) {
            return None;
        }
        let manifest = read_json(&directory.join("manifest.json")).ok()?;
        if manifest["name"] != "nero" || manifest["version"] != version {
            return None;
        }
        let settings = root.join("Claude Extensions Settings");
        if !plain_tree(root, &settings) {
            return None;
        }
        let settings = read_json(&settings.join("local.mcpb.nero.nero.json")).ok()?;
        let enabled = settings["isEnabled"].as_bool()?;
        Some(HostInstallation::Installed {
            directory,
            version: version.into(),
            enabled,
        })
    })();
    result.unwrap_or(HostInstallation::Unknown(
        "Claude의 설치 정보를 확인하지 못했습니다. Claude에서 Nero 확장 상태를 확인해 주세요.",
    ))
}

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !matches!(value, "." | "..")
}

fn plain_tree(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_owned();
    if !super::plain_directory(&current) {
        return false;
    }
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return false;
        };
        current.push(part);
        if !super::plain_directory(&current) {
            return false;
        }
    }
    true
}

fn installed_payload(
    directory: &Path,
    client: &str,
    root: &Path,
    connection: &Path,
    registration: &Value,
) -> Result<(String, String), String> {
    let config = if client == "claude" {
        read_json(&directory.join("manifest.json"))?["server"]["mcp_config"].clone()
    } else {
        read_json(&directory.join(".mcp.json"))?["mcpServers"]["nero"].clone()
    };
    let environment = &config["env"];
    let generation = environment["NERO_CONNECTION_GENERATION"]
        .as_str()
        .unwrap_or("");
    if !super::valid_generation(generation)
        || config["args"] != serde_json::json!(["--data-dir", root, "mcp"])
        || environment["NERO_CONNECTION_CLIENT"] != client
        || environment["NERO_CONNECTION_TOKEN"] != registration["token"]
        || environment["NERO_CONNECTION_DIR"] != serde_json::json!(connection.join("sessions"))
        || environment
            .as_object()
            .map_or(true, |values| values.len() != 4)
    {
        return Err("설치된 연결의 대상이 현재 Nero와 일치하지 않습니다.".into());
    }
    let server = directory.join("server");
    if !plain_tree(directory, &server) {
        return Err("설치된 연결 실행 파일을 확인하지 못했습니다.".into());
    }
    let binary = bounded_read(&server.join(executable_name()), 64 * 1024 * 1024)
        .map_err(|_| "설치된 연결 실행 파일을 읽지 못했습니다.".to_string())?;
    if binary.is_empty() {
        return Err("설치된 연결 실행 파일을 확인하지 못했습니다.".into());
    }
    if client == "codex" {
        let portable = read_json(&directory.join("mcp.json"))?;
        let entry = &portable["mcpServers"]["nero"];
        if entry["type"] != "stdio"
            || entry["command"] != format!("./server/{}", executable_name())
            || entry["args"] != config["args"]
            || entry["env"] != *environment
        {
            return Err("설치된 연결 설정을 확인하지 못했습니다.".into());
        }
        let expected = connection
            .join("releases")
            .join(generation)
            .join("plugins/nero/server")
            .join(executable_name());
        if config["command"] != serde_json::json!(expected) {
            return Err("설치된 연결 실행 경로가 일치하지 않습니다.".into());
        }
        if !plain_tree(connection, expected.parent().unwrap_or(connection))
            || !bounded_read(&expected, 64 * 1024 * 1024).is_ok_and(|bytes| bytes == binary)
        {
            return Err("설치된 연결과 실행 파일이 일치하지 않습니다.".into());
        }
    } else if config["command"] != format!("${{__dirname}}/server/{}", executable_name()) {
        return Err("설치된 연결 실행 경로가 일치하지 않습니다.".into());
    }
    let payload = payload_generation(client, &binary, |name| {
        let relative = match (client, name) {
            ("claude", "claude-manifest.json") => "manifest.json",
            ("codex", "codex-plugin.json") => "plugin.json",
            ("codex", "codex-legacy-plugin.json") => ".codex-plugin/plugin.json",
            ("codex", "meeting-summary/SKILL.md") => "skills/meeting-summary/SKILL.md",
            ("codex", "icon.png") => "assets/icon.png",
            (_, name) => name,
        };
        let path = directory.join(relative);
        if !plain_tree(directory, path.parent().unwrap_or(directory)) {
            return Err("설치된 연결 파일 경로를 확인하지 못했습니다.".into());
        }
        bounded_read(&path, 4 * 1024 * 1024)
            .map_err(|_| "설치된 연결 파일을 읽지 못했습니다.".into())
    })?;
    Ok((payload, generation.into()))
}

pub(super) fn client_info(
    client: &str,
    name: &str,
    resources: &Path,
    root: &Path,
    directory: &Path,
    registration: Option<&Value>,
    generation: Option<&str>,
    active: &[String],
    installation: Option<&HostInstallation>,
) -> ClientInfo {
    let version_name = if client == "claude" {
        "claude-manifest.json"
    } else {
        "codex-plugin.json"
    };
    let package_version = read_json(&resources.join("agent/integrations").join(version_name))
        .ok()
        .and_then(|value| value["version"].as_str().map(str::to_owned));
    let mut info = ClientInfo {
        id: client.into(),
        name: name.into(),
        state: "installation_unknown".into(),
        detail: None,
        installation_state: "unknown".into(),
        session_state: "inactive".into(),
        action: "recheck".into(),
        package_version,
        installed_version: None,
    };
    let Some(generation) = generation else {
        info.state = "unavailable".into();
        info.action = "none".into();
        return info;
    };
    let mut installed_generation = None;
    match (installation, registration) {
        (Some(HostInstallation::Missing), _) => info.installation_state = "not_installed".into(),
        (
            Some(HostInstallation::Installed {
                directory: installed,
                version,
                enabled,
            }),
            Some(registration),
        ) => {
            info.installed_version = Some(version.clone());
            match installed_payload(installed, client, root, directory, registration) {
                Ok((payload, receipt)) => {
                    info.installation_state = if !enabled {
                        "disabled"
                    } else if payload == generation {
                        "current"
                    } else {
                        "outdated"
                    }
                    .into();
                    if payload == generation {
                        installed_generation = Some(receipt);
                    }
                }
                Err(reason) => info.detail = Some(reason),
            }
        }
        (Some(HostInstallation::Unknown(reason)), _) => info.detail = Some((*reason).into()),
        _ => (),
    }
    let current_live = active
        .iter()
        .any(|value| value == generation || installed_generation.as_ref() == Some(value));
    info.session_state = if current_live {
        "current"
    } else if active.is_empty() {
        "inactive"
    } else {
        "outdated"
    }
    .into();
    match info.installation_state.as_str() {
        "outdated" => {
            info.state = "update_required".into();
            info.action = "update".into();
            info.detail = Some(format!("{name}에 설치된 Nero 연동을 업데이트해 주세요."));
        }
        "disabled" => {
            info.state = "disabled".into();
            info.detail = Some(format!("{name}에서 Nero 연동을 켠 뒤 다시 확인해 주세요."));
        }
        _ if current_live => {
            info.state = "connected".into();
            info.detail = None;
            info.action = "none".into();
        }
        "current" => {
            if active.is_empty() {
                info.state = "waiting".into();
                info.detail = Some(format!(
                    "연동은 최신입니다. {name}에서 Nero 연결을 시작한 뒤 다시 확인해 주세요."
                ));
            } else {
                info.state = "reconnect_required".into();
                info.detail = Some(format!("연동은 최신입니다. {name}의 이전 연결이 실행 중이므로 {name}를 다시 시작한 뒤 확인해 주세요."));
            }
        }
        "not_installed" => {
            info.state = "not_connected".into();
            info.action = "connect".into();
        }
        _ if registration.is_none() => {
            info.state = "not_connected".into();
            info.action = "connect".into();
        }
        _ if active.is_empty() && matches!(installation, Some(HostInstallation::Unknown(_))) => {
            info.action = "connect".into();
            info.detail = Some(format!(
                "{name}의 설치 상태를 확인하지 못했습니다. 연결 화면에서 설치 상태를 확인해 주세요."
            ));
        }
        _ => {
            if info.detail.is_none() {
                info.detail = Some(format!("{name}의 설치 상태를 확인하지 못했습니다. {name}에서 Nero 설치 상태를 확인해 주세요."));
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    #[test]
    fn native_probe_fixture() {
        use std::io::Write;
        match std::env::var("NERO_PROBE_TEST_MODE").as_deref() {
            Ok("timeout") => std::thread::sleep(Duration::from_secs(10)),
            Ok("oversized") => {
                let _ = std::io::stdout().write_all(&vec![b'x'; 2 * 1024 * 1024 + 1]);
            }
            _ => (),
        }
    }

    #[test]
    fn native_probe_has_a_deadline_and_output_bound_without_pipe_readers() {
        for mode in ["timeout", "oversized"] {
            let executable = std::env::current_exe().unwrap();
            assert!(native_executable(&executable).is_some());
            let mut command = Command::new(&executable);
            command
                .args([
                    "--exact",
                    &format!(
                        "{}::native_probe_fixture",
                        module_path!().split_once("::").unwrap().1
                    ),
                    "--nocapture",
                ])
                .env("NERO_PROBE_TEST_MODE", mode)
                .stdin(Stdio::null())
                .stderr(Stdio::null());
            let started = Instant::now();
            assert!(codex_list_output(command).is_err());
            assert!(started.elapsed() < Duration::from_secs(6));
        }
    }

    #[test]
    fn script_launchers_are_not_executed_during_host_observation() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = temp.path().join("codex");
        fs::write(&launcher, b"#!/bin/sh\nsleep 10 &\nwait\n").unwrap();
        assert!(native_executable(&launcher).is_none());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_native_fallback_requires_the_official_bundle_identity() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("ChatGPT.app");
        let native = bundle.join("Contents/Resources/codex-cli/bin/codex");
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        symlink(std::env::current_exe().unwrap(), &native).unwrap();
        let info = bundle.join("Contents/Info.plist");
        for (identifier, accepted) in [("com.example.unrelated", false), ("com.openai.codex", true)]
        {
            fs::write(&info, format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{identifier}</string></dict></plist>")).unwrap();
            assert_eq!(codex_desktop_executable(&bundle).is_some(), accepted);
        }
        let moved = bundle.join("Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&native, &moved).unwrap();
        assert!(codex_desktop_executable(&bundle).is_some());
    }

    fn missing_hosts() -> HostInstallations {
        HostInstallations {
            claude: HostInstallation::Missing,
            codex: HostInstallation::Missing,
        }
    }

    fn prepared_codex_catalog(root: &Path) -> PathBuf {
        let connection = root.join("connections/codex");
        fs::create_dir_all(connection.join(".agents/plugins")).unwrap();
        let source = format!("./releases/{}/plugins/nero", "a".repeat(64));
        fs::create_dir_all(connection.join(source.trim_start_matches("./"))).unwrap();
        fs::write(connection.join("state.json"), b"preserve registration").unwrap();
        fs::create_dir_all(connection.join("releases/previous/server")).unwrap();
        fs::write(
            connection.join("releases/previous/server/nero"),
            b"preserve previous runtime",
        )
        .unwrap();
        fs::write(connection.join(".agents/plugins/marketplace.json"), json!({"name":"nero-local","plugins":[{"name":"nero","source":{"source":"local","path":source}}]}).to_string()).unwrap();
        connection
    }

    #[test]
    fn explicit_codex_installation_updates_only_nero_in_order_and_preserves_prepared_files() {
        let temp = tempfile::tempdir().unwrap();
        let connection = prepared_codex_catalog(temp.path());
        let catalog = fs::read(connection.join(".agents/plugins/marketplace.json")).unwrap();
        let mut calls = Vec::new();
        install_codex_with_runner(&connection, |arguments| {
            calls.push(arguments.to_vec());
            Ok(br#"{"ok":true}"#.to_vec())
        })
        .unwrap();
        assert_eq!(
            calls,
            vec![
                vec![
                    OsString::from("plugin"),
                    "marketplace".into(),
                    "add".into(),
                    connection.as_os_str().to_owned(),
                    "--json".into()
                ],
                vec![
                    "plugin".into(),
                    "add".into(),
                    "nero@nero-local".into(),
                    "--json".into()
                ],
            ]
        );
        assert_eq!(
            fs::read(connection.join("state.json")).unwrap(),
            b"preserve registration"
        );
        assert_eq!(
            fs::read(connection.join("releases/previous/server/nero")).unwrap(),
            b"preserve previous runtime"
        );
        assert_eq!(
            fs::read(connection.join(".agents/plugins/marketplace.json")).unwrap(),
            catalog
        );
    }

    #[test]
    fn failed_or_uncertain_codex_installation_stops_without_rollback_or_automatic_retry() {
        for failed_step in 0..2 {
            let temp = tempfile::tempdir().unwrap();
            let connection = prepared_codex_catalog(temp.path());
            let mut calls = 0;
            let error = install_codex_with_runner(&connection, |_| {
                let step = calls;
                calls += 1;
                if step == failed_step {
                    Err("untrusted diagnostic".into())
                } else {
                    Ok(br#"{"ok":true}"#.to_vec())
                }
            })
            .unwrap_err();
            assert_eq!(calls, failed_step + 1);
            assert!(error.contains("다시 확인"));
            assert!(!error.contains("untrusted"));
            assert_eq!(
                fs::read(connection.join("state.json")).unwrap(),
                b"preserve registration"
            );
            assert_eq!(
                fs::read(connection.join("releases/previous/server/nero")).unwrap(),
                b"preserve previous runtime"
            );
        }
        let temp = tempfile::tempdir().unwrap();
        let connection = prepared_codex_catalog(temp.path());
        let mut calls = 0;
        assert!(install_codex_with_runner(&connection, |_| {
            calls += 1;
            Ok(b"unknown success output".to_vec())
        })
        .is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn explicit_codex_installation_rejects_catalogs_containing_other_plugins() {
        let temp = tempfile::tempdir().unwrap();
        let connection = prepared_codex_catalog(temp.path());
        let path = connection.join(".agents/plugins/marketplace.json");
        let mut catalog = read_json(&path).unwrap();
        catalog["plugins"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"unrelated"}));
        fs::write(path, catalog.to_string()).unwrap();
        assert!(
            install_codex_with_runner(&connection, |_| panic!("must not run a host mutation"))
                .is_err()
        );
    }

    #[test]
    fn host_refresh_uses_a_bounded_cache_and_only_explicit_force_skips_it() {
        let temp = tempfile::tempdir().unwrap();
        let roots = (
            temp.path().join("home"),
            temp.path().join("config"),
            temp.path().join("connections"),
            temp.path().join("codex"),
        );
        let mut cache = None;
        let inspected = Cell::new(0);
        let inspect = || {
            inspected.set(inspected.get() + 1);
            missing_hosts()
        };
        for _ in 0..10 {
            probe_cached(&mut cache, roots.clone(), false, inspect);
        }
        assert_eq!(inspected.get(), 1);
        probe_cached(&mut cache, roots.clone(), true, inspect);
        assert_eq!(inspected.get(), 2);
        cache.as_mut().unwrap().observed = Instant::now() - Duration::from_secs(31);
        probe_cached(&mut cache, roots.clone(), false, inspect);
        assert_eq!(inspected.get(), 3);
        let mut different_roots = roots;
        different_roots.3 = temp.path().join("other codex home");
        probe_cached(&mut cache, different_roots, false, inspect);
        assert_eq!(inspected.get(), 4);
        assert!(!temp.path().join("home").exists());
    }

    fn codex_fixture(root: &Path) -> (PathBuf, PathBuf, Value) {
        let home = root.join("codex home");
        let connection = root.join("connections/codex");
        let version = "0.0.1-nero.22";
        let generation = "a".repeat(64);
        let directory = home.join("plugins/cache/nero-local/nero").join(version);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("plugin.json"),
            json!({"name":"nero","version":version}).to_string(),
        )
        .unwrap();
        fs::write(
            directory.join(".mcp.json"),
            json!({"mcpServers":{"nero":{"env":{"NERO_CONNECTION_GENERATION":generation}}}})
                .to_string(),
        )
        .unwrap();
        let entry = json!({
            "pluginId":"nero@nero-local", "name":"nero", "marketplaceName":"nero-local",
            "installed":true, "enabled":true, "version":version,
            "source":{"path":connection.join("releases").join(generation).join("plugins/nero")}
        });
        (home, connection, json!({"installed":[entry]}))
    }

    #[test]
    fn codex_observation_requires_official_entry_and_matching_cache_manifest_and_source() {
        let temp = tempfile::tempdir().unwrap();
        let (home, connection, value) = codex_fixture(temp.path());
        assert!(matches!(
            codex_installation(value.to_string().as_bytes(), &home, &connection),
            HostInstallation::Installed { enabled: true, .. }
        ));
        let mut disabled = value.clone();
        disabled["installed"][0]["enabled"] = json!(false);
        assert!(matches!(
            codex_installation(disabled.to_string().as_bytes(), &home, &connection),
            HostInstallation::Installed { enabled: false, .. }
        ));
        assert!(matches!(
            codex_installation(br#"{"installed":[]}"#, &home, &connection),
            HostInstallation::Missing
        ));
        for changed in ["source", "generation", "version", "identity", "duplicate"] {
            let mut invalid = value.clone();
            match changed {
                "source" => {
                    invalid["installed"][0]["source"]["path"] = json!(temp.path().join("outside"))
                }
                "generation" => {
                    invalid["installed"][0]["source"]["path"] = json!(connection
                        .join("releases")
                        .join("b".repeat(64))
                        .join("plugins/nero"))
                }
                "version" => invalid["installed"][0]["version"] = json!("../outside"),
                "identity" => invalid["installed"][0]["name"] = json!("different plugin"),
                "duplicate" => {
                    let duplicate = invalid["installed"][0].clone();
                    invalid["installed"].as_array_mut().unwrap().push(duplicate);
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    codex_installation(invalid.to_string().as_bytes(), &home, &connection),
                    HostInstallation::Unknown(_)
                ),
                "{changed}"
            );
        }
        fs::write(
            home.join("plugins/cache/nero-local/nero/0.0.1-nero.22/plugin.json"),
            json!({"name":"nero","version":"other"}).to_string(),
        )
        .unwrap();
        assert!(matches!(
            codex_installation(value.to_string().as_bytes(), &home, &connection),
            HostInstallation::Unknown(_)
        ));
    }

    #[test]
    fn claude_observation_requires_local_installation_manifest_and_enabled_setting() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Claude");
        assert!(matches!(
            claude_installation(&root),
            HostInstallation::Missing
        ));
        fs::create_dir_all(root.join("Claude Extensions/local.mcpb.nero.nero")).unwrap();
        fs::create_dir_all(root.join("Claude Extensions Settings")).unwrap();
        let installations = root.join("extensions-installations.json");
        fs::write(&installations, json!({"extensions":{"local.mcpb.nero.nero":{"id":"local.mcpb.nero.nero","version":"0.0.1-nero.22"}}}).to_string()).unwrap();
        let manifest = root.join("Claude Extensions/local.mcpb.nero.nero/manifest.json");
        fs::write(
            &manifest,
            json!({"name":"nero","version":"0.0.1-nero.22"}).to_string(),
        )
        .unwrap();
        let settings = root.join("Claude Extensions Settings/local.mcpb.nero.nero.json");
        fs::write(&settings, json!({"isEnabled":true}).to_string()).unwrap();
        assert!(matches!(
            claude_installation(&root),
            HostInstallation::Installed { enabled: true, .. }
        ));
        fs::write(&settings, json!({"isEnabled":false}).to_string()).unwrap();
        assert!(matches!(
            claude_installation(&root),
            HostInstallation::Installed { enabled: false, .. }
        ));
        fs::write(
            &manifest,
            json!({"name":"nero","version":"different"}).to_string(),
        )
        .unwrap();
        assert!(matches!(
            claude_installation(&root),
            HostInstallation::Unknown(_)
        ));
        fs::write(&installations, b"invalid JSON").unwrap();
        assert!(matches!(
            claude_installation(&root),
            HostInstallation::Unknown(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn linked_codex_cache_is_unknown_and_does_not_read_or_write_the_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let (home, connection, value) = codex_fixture(temp.path());
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), b"preserve external file").unwrap();
        let cached = home.join("plugins/cache/nero-local/nero/0.0.1-nero.22");
        fs::rename(&cached, temp.path().join("preserved cache")).unwrap();
        symlink(&external, &cached).unwrap();
        assert!(matches!(
            codex_installation(value.to_string().as_bytes(), &home, &connection),
            HostInstallation::Unknown(_)
        ));
        assert_eq!(
            fs::read(external.join("keep")).unwrap(),
            b"preserve external file"
        );
        assert_eq!(fs::read_dir(external).unwrap().count(), 1);
    }
}
