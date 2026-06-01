use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

const ENABLE_ENV: &str = "YAZELIX_HELIX_BRIDGE";
const STATE_DIR_ENV: &str = "YAZELIX_STATE_DIR";
const SESSION_ID_ENV: &str = "YAZELIX_HELIX_BRIDGE_SESSION_ID";
const INSTANCE_ID_ENV: &str = "YAZELIX_HELIX_BRIDGE_INSTANCE_ID";
const AUTH_TOKEN_ENV: &str = "YAZELIX_HELIX_BRIDGE_AUTH_TOKEN";
const MANAGED_CONFIG_ENV: &str = "YAZELIX_HELIX_MANAGED_CONFIG_PATH";
const BRIDGE_SCHEMA_VERSION: u8 = 2;
const DEFAULT_TIMEOUT_MS: u64 = 1_500;
const MAX_TIMEOUT_MS: u64 = 10_000;

pub(crate) struct BridgeRuntime {
    pub(crate) bridge: Option<YazelixBridge>,
    pub(crate) requests: UnboundedReceiver<BridgeCommand>,
    pub(crate) _disabled_sender_guard: Option<UnboundedSender<BridgeCommand>>,
}

pub(crate) struct YazelixBridge {
    transport: BridgeTransport,
    registry_path: PathBuf,
    token_path: PathBuf,
}

pub(crate) struct BridgeCommand {
    pub(crate) request_id: String,
    pub(crate) action: String,
    pub(crate) payload: Value,
    response_tx: mpsc::Sender<BridgeResponse>,
}

#[derive(Debug, Clone)]
struct BridgeConfig {
    state_dir: PathBuf,
    session_id: String,
    instance_id: String,
    auth_token: String,
    managed_config_path: Option<String>,
    zellij_session_name: Option<String>,
    zellij_tab_position: Option<String>,
    zellij_pane_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BridgeRequest {
    schema_version: u8,
    request_id: String,
    auth_token: String,
    action: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct BridgeResponse {
    schema_version: u8,
    request_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BridgeError>,
}

#[derive(Debug, Serialize)]
struct BridgeError {
    class: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BridgeTransport {
    UnixSocket { path: String },
    WindowsNamedPipe { name: String },
}

#[derive(Debug, Serialize)]
struct BridgeRegistry {
    schema_version: u8,
    session_id: String,
    instance_id: String,
    transport: BridgeTransport,
    auth_token_path: String,
    pid: u32,
    zellij_session_name: Option<String>,
    zellij_tab_position: Option<String>,
    zellij_pane_id: Option<String>,
    started_at_unix_ms: u128,
    managed_config_path: Option<String>,
}

impl BridgeRuntime {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let (tx, requests) = unbounded_channel();
        let Some(config) = BridgeConfig::from_env()? else {
            return Ok(Self {
                bridge: None,
                requests,
                _disabled_sender_guard: Some(tx),
            });
        };

        let bridge = YazelixBridge::start(config, tx)?;
        Ok(Self {
            bridge: Some(bridge),
            requests,
            _disabled_sender_guard: None,
        })
    }
}

impl BridgeCommand {
    pub(crate) fn respond_ok(self, data: Value) {
        let _ = self
            .response_tx
            .send(BridgeResponse::ok(self.request_id, data));
    }

    pub(crate) fn respond_error(self, class: &'static str, message: impl Into<String>) {
        let _ = self
            .response_tx
            .send(BridgeResponse::error(self.request_id, class, message));
    }
}

impl BridgeResponse {
    pub(crate) fn ok(request_id: String, data: Value) -> Self {
        Self {
            schema_version: BRIDGE_SCHEMA_VERSION,
            request_id,
            status: "ok",
            data: Some(data),
            error: None,
        }
    }

    fn error(
        request_id: impl Into<String>,
        class: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: BRIDGE_SCHEMA_VERSION,
            request_id: request_id.into(),
            status: "error",
            data: None,
            error: Some(BridgeError {
                class,
                message: message.into(),
            }),
        }
    }
}

impl BridgeConfig {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let enabled = match std::env::var(ENABLE_ENV) {
            Ok(value) if matches_enabled(&value) => true,
            Ok(value) if matches_disabled(&value) => false,
            Ok(value) => anyhow::bail!(
                "{ENABLE_ENV} must be one of 1, true, yes, on, 0, false, no, or off; got `{value}`"
            ),
            Err(std::env::VarError::NotPresent) => false,
            Err(err) => anyhow::bail!("Could not read {ENABLE_ENV}: {err}"),
        };

        if !enabled {
            return Ok(None);
        }

        let state_dir = required_path_env(STATE_DIR_ENV)?;
        let session_id = required_id_env(SESSION_ID_ENV)?;
        let instance_id = std::env::var(INSTANCE_ID_ENV)
            .ok()
            .map(|value| validate_id(INSTANCE_ID_ENV, value))
            .transpose()?
            .unwrap_or_else(default_instance_id);
        let auth_token = required_secret_env(AUTH_TOKEN_ENV)?;

        Ok(Some(Self {
            state_dir,
            session_id,
            instance_id,
            auth_token,
            managed_config_path: std::env::var(MANAGED_CONFIG_ENV).ok(),
            zellij_session_name: std::env::var("ZELLIJ_SESSION_NAME").ok(),
            zellij_tab_position: std::env::var("ZELLIJ_TAB_POSITION").ok(),
            zellij_pane_id: std::env::var("ZELLIJ_PANE_ID").ok(),
        }))
    }
}

impl YazelixBridge {
    fn start(config: BridgeConfig, tx: UnboundedSender<BridgeCommand>) -> anyhow::Result<Self> {
        let bridge_dir = config
            .state_dir
            .join("helix_bridge")
            .join(&config.session_id);
        fs::create_dir_all(&bridge_dir)?;
        set_owner_only_dir_permissions(&bridge_dir)?;

        let registry_path = bridge_dir.join(format!("{}.json", config.instance_id));
        let token_path = bridge_dir.join(format!("{}.token", config.instance_id));
        let transport = bridge_transport(&config, &bridge_dir)?;

        write_private_file(&token_path, &config.auth_token)?;

        let registry = BridgeRegistry {
            schema_version: BRIDGE_SCHEMA_VERSION,
            session_id: config.session_id.clone(),
            instance_id: config.instance_id.clone(),
            transport: transport.clone(),
            auth_token_path: token_path.display().to_string(),
            pid: std::process::id(),
            zellij_session_name: config.zellij_session_name.clone(),
            zellij_tab_position: config.zellij_tab_position.clone(),
            zellij_pane_id: config.zellij_pane_id.clone(),
            started_at_unix_ms: started_at_unix_ms(),
            managed_config_path: config.managed_config_path.clone(),
        };
        write_private_file(&registry_path, &serde_json::to_string_pretty(&registry)?)?;

        start_listener(transport.clone(), config.auth_token, tx)?;

        Ok(Self {
            transport,
            registry_path,
            token_path,
        })
    }
}

impl Drop for YazelixBridge {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let BridgeTransport::UnixSocket { path } = &self.transport {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(&self.registry_path);
        let _ = fs::remove_file(&self.token_path);
    }
}

#[cfg(unix)]
fn bridge_transport(config: &BridgeConfig, bridge_dir: &Path) -> anyhow::Result<BridgeTransport> {
    let socket_path = bridge_dir.join(format!("{}.sock", config.instance_id));
    remove_stale_file(&socket_path)?;
    Ok(BridgeTransport::UnixSocket {
        path: socket_path.display().to_string(),
    })
}

#[cfg(windows)]
fn bridge_transport(config: &BridgeConfig, _bridge_dir: &Path) -> anyhow::Result<BridgeTransport> {
    Ok(BridgeTransport::WindowsNamedPipe {
        name: format!(
            r"\\.\pipe\yazelix-helix-{}-{}",
            config.session_id, config.instance_id
        ),
    })
}

#[cfg(not(any(unix, windows)))]
fn bridge_transport(_config: &BridgeConfig, _bridge_dir: &Path) -> anyhow::Result<BridgeTransport> {
    anyhow::bail!("Yazelix Helix bridge requires Unix sockets or Windows named pipes")
}

#[cfg(unix)]
fn start_listener(
    transport: BridgeTransport,
    auth_token: String,
    tx: UnboundedSender<BridgeCommand>,
) -> anyhow::Result<()> {
    use std::os::unix::net::UnixListener;

    let BridgeTransport::UnixSocket { path } = transport else {
        anyhow::bail!("Unix Helix bridge listener received a non-Unix transport")
    };
    let socket_path = PathBuf::from(path);
    let listener = UnixListener::bind(&socket_path)?;
    std::thread::Builder::new()
        .name("yazelix-helix-bridge".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(2_000)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(2_000)));
                        handle_connection(&mut stream, &auth_token, &tx);
                    }
                    Err(err) => {
                        log::warn!("Yazelix Helix bridge listener failed: {err}");
                        break;
                    }
                }
            }
        })?;

    Ok(())
}

#[cfg(windows)]
fn start_listener(
    transport: BridgeTransport,
    auth_token: String,
    tx: UnboundedSender<BridgeCommand>,
) -> anyhow::Result<()> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_ACCESS_DUPLEX,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    let BridgeTransport::WindowsNamedPipe { name } = transport else {
        anyhow::bail!("Windows Helix bridge listener received a non-Windows transport")
    };
    let pipe_name = windows_wide(&name);
    std::thread::Builder::new()
        .name("yazelix-helix-bridge".into())
        .spawn(move || loop {
            let handle = unsafe {
                CreateNamedPipeW(
                    pipe_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    8192,
                    8192,
                    0,
                    std::ptr::null(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                log::warn!(
                    "Yazelix Helix bridge failed to create named pipe: {}",
                    std::io::Error::last_os_error()
                );
                break;
            }

            let connected = unsafe {
                ConnectNamedPipe(handle, std::ptr::null_mut()) != 0
                    || GetLastError() == ERROR_PIPE_CONNECTED
            };
            if !connected {
                log::warn!(
                    "Yazelix Helix bridge named pipe connect failed: {}",
                    std::io::Error::last_os_error()
                );
                unsafe {
                    CloseHandle(handle);
                }
                continue;
            }

            let mut pipe = WindowsPipeHandle(handle);
            handle_connection(&mut pipe, &auth_token, &tx);
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
        })?;

    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn start_listener(
    _transport: BridgeTransport,
    _auth_token: String,
    _tx: UnboundedSender<BridgeCommand>,
) -> anyhow::Result<()> {
    anyhow::bail!("Yazelix Helix bridge requires Unix sockets or Windows named pipes")
}

fn handle_connection<S>(stream: &mut S, auth_token: &str, tx: &UnboundedSender<BridgeCommand>)
where
    S: Read + Write,
{
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut *stream);
        if let Err(err) = reader.read_line(&mut line) {
            write_response(
                stream,
                BridgeResponse::error(
                    "",
                    "invalid_payload",
                    format!("Could not read request: {err}"),
                ),
            );
            return;
        }
    }

    let request = match parse_request(&line, auth_token) {
        Ok(request) => request,
        Err(response) => {
            write_response(stream, response);
            return;
        }
    };

    let timeout = Duration::from_millis(
        request
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS),
    );
    let (response_tx, response_rx) = mpsc::channel();
    let request_id = request.request_id.clone();
    if tx
        .send(BridgeCommand {
            request_id,
            action: request.action,
            payload: request.payload,
            response_tx,
        })
        .is_err()
    {
        write_response(
            stream,
            BridgeResponse::error(
                request.request_id,
                "stale_instance",
                "Helix bridge is no longer accepting requests",
            ),
        );
        return;
    }

    match response_rx.recv_timeout(timeout) {
        Ok(response) => write_response(stream, response),
        Err(mpsc::RecvTimeoutError::Timeout) => write_response(
            stream,
            BridgeResponse::error(
                request.request_id,
                "timeout",
                "Timed out waiting for Helix to handle the bridge request",
            ),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => write_response(
            stream,
            BridgeResponse::error(
                request.request_id,
                "stale_instance",
                "Helix bridge response channel closed",
            ),
        ),
    }
}

fn parse_request(line: &str, auth_token: &str) -> Result<BridgeRequest, BridgeResponse> {
    let request: BridgeRequest = serde_json::from_str(line).map_err(|err| {
        BridgeResponse::error(
            "",
            "invalid_payload",
            format!("Invalid JSON request: {err}"),
        )
    })?;
    if request.schema_version != BRIDGE_SCHEMA_VERSION {
        return Err(BridgeResponse::error(
            request.request_id,
            "invalid_payload",
            format!(
                "Unsupported bridge schema version {}; expected {BRIDGE_SCHEMA_VERSION}",
                request.schema_version
            ),
        ));
    }
    if request.request_id.trim().is_empty() {
        return Err(BridgeResponse::error(
            "",
            "invalid_payload",
            "Bridge request_id must be non-empty",
        ));
    }
    if request.action.trim().is_empty() {
        return Err(BridgeResponse::error(
            request.request_id,
            "invalid_payload",
            "Bridge action must be non-empty",
        ));
    }
    if request.auth_token != auth_token {
        return Err(BridgeResponse::error(
            request.request_id,
            "permission_denied",
            "Bridge auth token did not match this Helix instance",
        ));
    }
    Ok(request)
}

fn write_response(stream: &mut impl Write, response: BridgeResponse) {
    match serde_json::to_string(&response) {
        Ok(encoded) => {
            let _ = writeln!(stream, "{encoded}");
        }
        Err(err) => {
            let fallback = json!({
                "schema_version": BRIDGE_SCHEMA_VERSION,
                "request_id": "",
                "status": "error",
                "error": {
                    "class": "internal_error",
                    "message": format!("Could not serialize bridge response: {err}")
                }
            });
            let _ = writeln!(stream, "{fallback}");
        }
    }
}

#[cfg(windows)]
struct WindowsPipeHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsPipeHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
impl Read for WindowsPipeHandle {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::ReadFile;

        let mut bytes_read = 0u32;
        let ok = unsafe {
            ReadFile(
                self.0,
                buffer.as_mut_ptr().cast(),
                buffer.len().min(u32::MAX as usize) as u32,
                &mut bytes_read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(bytes_read as usize)
    }
}

#[cfg(windows)]
impl Write for WindowsPipeHandle {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;

        let mut bytes_written = 0u32;
        let ok = unsafe {
            WriteFile(
                self.0,
                buffer.as_ptr().cast(),
                buffer.len().min(u32::MAX as usize) as u32,
                &mut bytes_written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(bytes_written as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn windows_wide(value: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn matches_enabled(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "yes" | "on")
}

fn matches_disabled(value: &str) -> bool {
    matches!(value.trim(), "0" | "false" | "no" | "off")
}

fn required_path_env(name: &str) -> anyhow::Result<PathBuf> {
    let value = std::env::var(name)?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} must be non-empty when {ENABLE_ENV}=1");
    }
    Ok(PathBuf::from(value))
}

fn required_id_env(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name)?;
    validate_id(name, value)
}

fn validate_id(name: &str, value: String) -> anyhow::Result<String> {
    if value.is_empty() || value.trim() != value {
        anyhow::bail!("{name} must be non-empty and untrimmed");
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        anyhow::bail!(
            "{name} may only contain ASCII letters, numbers, dots, hyphens, and underscores"
        );
    }
    Ok(value)
}

fn required_secret_env(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name)?;
    if value.is_empty() || value.trim() != value || value.contains(['\n', '\r']) {
        anyhow::bail!("{name} must be non-empty, untrimmed, and single-line");
    }
    Ok(value)
}

fn default_instance_id() -> String {
    format!("hx-{}-{}", std::process::id(), started_at_unix_ms())
}

fn started_at_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn remove_stale_file(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn write_private_file(path: &Path, content: &str) -> anyhow::Result<()> {
    fs::write(path, content)?;
    set_owner_only_file_permissions(path)
}

#[cfg(unix)]
fn set_owner_only_dir_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_dir_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_file_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

// Test lane: default
#[cfg(test)]
mod tests {
    use super::*;

    // Defends: the bridge rejects cross-instance requests before they reach the editor event loop.
    #[test]
    fn parse_request_rejects_wrong_auth_token() {
        let response = parse_request(
            r#"{"schema_version":2,"request_id":"r1","auth_token":"wrong","action":"helix.get_context"}"#,
            "expected",
        )
        .unwrap_err();

        assert_eq!(response.status, "error");
        assert_eq!(response.error.unwrap().class, "permission_denied");
    }

    // Defends: bridge path components are safe filesystem names, not relative paths.
    #[test]
    fn validate_id_rejects_path_traversal() {
        let err = validate_id("TEST_ID", "../session".to_string()).unwrap_err();
        assert!(err.to_string().contains("ASCII letters"));
    }
}
