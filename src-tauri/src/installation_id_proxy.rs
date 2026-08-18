//! Local relay for account-scoped Codex device and session convergence.
//!
//! It changes only documented Codex identity carriers. Authentication and
//! unrelated request data are forwarded without interpretation.

use crate::models::AppError;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};
use tokio::sync::{Mutex, oneshot};

/// This is the built-in ChatGPT OAuth Codex endpoint, rather than the value
/// written to `openai_base_url`; using a fixed canonical upstream prevents a
/// localhost relay from ever forwarding back into itself.
pub(crate) const OFFICIAL_CODEX_UPSTREAM: &str = "https://chatgpt.com/backend-api/codex";
pub(crate) const RELAY_PATH_PREFIX: &str = "codex-tools-installation-id";
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct RelayTarget {
    pub(crate) base_url: String,
}

#[derive(Default)]
pub(crate) struct InstallationIdProxyRegistry {
    running: Mutex<Option<RunningRelay>>,
}

struct RunningRelay {
    shutdown: oneshot::Sender<()>,
}

struct RelayState {
    token: String,
    installation_id: String,
    session_id: String,
    account_id: String,
    client: RelayClient,
}

impl InstallationIdProxyRegistry {
    /// Starts a fresh, authenticated loopback listener for this activation.
    /// The unguessable token is part of the local base-url path, so Codex sends
    /// it without needing to override built-in OpenAI provider headers.
    pub(crate) async fn ensure(
        &self,
        installation_id: &str,
        session_id: &str,
        account_id: &str,
    ) -> Result<RelayTarget, AppError> {
        let mut running = self.running.lock().await;
        if let Some(previous) = running.take() {
            let _ = previous.shutdown.send(());
        }
        let token = format!(
            "{}{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| AppError::Internal(format!("无法启动本机安装标识中继：{error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| AppError::Internal(format!("无法读取本机安装标识中继端口：{error}")))?
            .port();
        let state = RelayState {
            token: token.clone(),
            installation_id: installation_id.to_owned(),
            session_id: session_id.to_owned(),
            account_id: account_id.to_owned(),
            client: RelayClient::default(),
        };
        let (shutdown, shutdown_rx) = oneshot::channel();
        let app = Router::new()
            .route("/codex-tools-installation-id/{token}/{*path}", any(relay))
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
            .with_state(Arc::new(state));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        *running = Some(RunningRelay { shutdown });
        Ok(RelayTarget {
            base_url: format!("http://127.0.0.1:{port}/{RELAY_PATH_PREFIX}/{token}"),
        })
    }

    pub(crate) async fn stop_all(&self) {
        if let Some(running) = self.running.lock().await.take() {
            let _ = running.shutdown.send(());
        }
    }
}

async fn relay(
    State(state): State<Arc<RelayState>>,
    Path((token, _path)): Path<(String, String)>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let ids = converged_request_ids(
        &headers,
        &state.installation_id,
        &state.session_id,
        &state.account_id,
    );
    let body = match rewrite_converged_request_body(&headers, body, &ids) {
        Ok(body) => body,
        Err(()) => return (StatusCode::BAD_REQUEST, "invalid JSON request body").into_response(),
    };
    let mut upstream_headers = reqwest::header::HeaderMap::new();
    for (name, value) in &headers {
        if is_hop_by_hop_header(&headers, name.as_str()) || name.as_str() == "host" {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("x-codex-turn-metadata") {
            upstream_headers.append(name.clone(), rewrite_turn_metadata_header(value, &ids));
        } else {
            upstream_headers.append(name.clone(), value.clone());
        }
    }
    apply_converged_id_headers(&mut upstream_headers, &ids);
    let Some(url) = upstream_url(&original_uri, &token) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let client = match state.client.current() {
        Ok(client) => client,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let response = match client
        .request(method, url)
        .headers(upstream_headers)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let status = response.status();
    let response_headers = response.headers().clone();
    let stream = response.bytes_stream().map_err(std::io::Error::other);
    let mut outgoing = Response::new(Body::from_stream(stream));
    *outgoing.status_mut() =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in &response_headers {
        if !is_hop_by_hop_header(&response_headers, name.as_str()) {
            outgoing.headers_mut().append(name.clone(), value.clone());
        }
    }
    outgoing
}

fn apply_converged_id_headers(headers: &mut HeaderMap, ids: &ConvergedRequestIds) {
    headers.insert(
        "x-codex-installation-id",
        HeaderValue::from_str(&ids.installation_id).expect("UUID is a valid header value"),
    );
    for (name, value) in [
        ("x-codex-window-id", ids.window_id.as_str()),
        ("x-client-request-id", ids.thread_id.as_str()),
        ("session-id", ids.session_id.as_str()),
        ("session_id", ids.session_id.as_str()),
        ("thread-id", ids.thread_id.as_str()),
    ] {
        headers.insert(
            name,
            HeaderValue::from_str(value).expect("generated ids are valid headers"),
        );
    }
}

/// Snapshot-aware system-proxy client. The relay follows the same environment
/// and platform proxy settings as the rest of Codex Tools without imposing a
/// total timeout on streaming Responses traffic.
#[derive(Default)]
struct RelayClient {
    cached: std::sync::Mutex<Option<(crate::network::ProxySnapshot, reqwest::Client)>>,
}

impl RelayClient {
    fn current(&self) -> Result<reqwest::Client, AppError> {
        let snapshot = crate::network::ClientCache::cached_snapshot();
        let mut cached = self
            .cached
            .lock()
            .map_err(|_| AppError::Internal("安装标识中继网络客户端锁已损坏。".into()))?;
        if let Some((previous, client)) = cached.as_ref()
            && previous == &snapshot
        {
            return Ok(client.clone());
        }
        let client = crate::network::ClientCache::build_standalone(None).map_err(|error| {
            AppError::Internal(format!("无法初始化安装标识中继网络客户端：{error}"))
        })?;
        *cached = Some((snapshot, client.clone()));
        Ok(client)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConvergedRequestIds {
    installation_id: String,
    session_id: String,
    thread_id: String,
    turn_id: String,
    window_id: String,
    turn_started_at_unix_ms: i64,
}

fn converged_request_ids(
    headers: &HeaderMap,
    installation_id: &str,
    session_id: &str,
    account_id: &str,
) -> ConvergedRequestIds {
    let original_session = headers
        .get("session-id")
        .or_else(|| headers.get("session_id"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(session_id);
    let thread_id = deterministic_uuid("codex-tools-thread-id-v1", &[account_id, original_session]);
    let turn_id = uuid::Uuid::now_v7().to_string();
    ConvergedRequestIds {
        installation_id: installation_id.into(),
        session_id: session_id.into(),
        window_id: format!("{thread_id}:0"),
        thread_id,
        turn_id,
        turn_started_at_unix_ms: chrono::Utc::now().timestamp_millis(),
    }
}

fn deterministic_uuid(domain: &str, fields: &[&str]) -> String {
    let mut bytes = Vec::from(domain.as_bytes());
    for field in fields {
        bytes.extend_from_slice(b"\0");
        bytes.extend_from_slice(field.as_bytes());
    }
    let digest = Sha256::digest(bytes);
    let mut uuid: [u8; 16] = digest[..16].try_into().expect("hash prefix");
    uuid[6] = (uuid[6] & 0x0f) | 0x40;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(uuid).to_string()
}

fn rewrite_converged_request_body(
    headers: &HeaderMap,
    body: Bytes,
    ids: &ConvergedRequestIds,
) -> Result<Bytes, ()> {
    let is_json = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("application/json"));
    if !is_json {
        return Ok(body);
    }
    let zstd = headers
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("zstd"));
    let decoded = if zstd {
        decode_zstd_body(&body)?
    } else {
        body.to_vec()
    };
    let mut value: Value = serde_json::from_slice(&decoded).map_err(|_| ())?;
    if let Some(object) = value.as_object_mut() {
        let metadata = object
            .entry("client_metadata")
            .or_insert_with(|| Value::Object(Map::new()));
        if !metadata.is_object() {
            *metadata = Value::Object(Map::new());
        }
        let metadata = metadata
            .as_object_mut()
            .expect("client metadata was normalized to an object");
        metadata.insert(
            "x-codex-installation-id".into(),
            Value::String(ids.installation_id.clone()),
        );
        metadata.insert("session_id".into(), Value::String(ids.session_id.clone()));
        metadata.insert("thread_id".into(), Value::String(ids.thread_id.clone()));
        metadata.insert("turn_id".into(), Value::String(ids.turn_id.clone()));
        metadata.insert(
            "x-codex-window-id".into(),
            Value::String(ids.window_id.clone()),
        );
        if let Some(turn_metadata) = metadata.get_mut("x-codex-turn-metadata") {
            rewrite_turn_metadata(turn_metadata, ids);
        }
    }
    let rendered = serde_json::to_vec(&value).map_err(|_| ())?;
    if zstd {
        zstd::stream::encode_all(rendered.as_slice(), 0)
            .map(Bytes::from)
            .map_err(|_| ())
    } else {
        Ok(Bytes::from(rendered))
    }
}

fn upstream_url(original_uri: &axum::http::Uri, token: &str) -> Option<String> {
    let prefix = format!("/{RELAY_PATH_PREFIX}/{token}");
    let suffix = original_uri
        .path_and_query()?
        .as_str()
        .strip_prefix(&prefix)?;
    if !suffix.starts_with('/') {
        return None;
    }
    Some(format!("{OFFICIAL_CODEX_UPSTREAM}{suffix}"))
}

fn is_hop_by_hop_header(headers: &HeaderMap, name: &str) -> bool {
    if matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    ) {
        return true;
    }
    headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case(name))
}

fn decode_zstd_body(body: &[u8]) -> Result<Vec<u8>, ()> {
    let decoder = zstd::stream::read::Decoder::new(body).map_err(|_| ())?;
    let mut decoded = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED_REQUEST_BODY_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|_| ())?;
    if decoded.len() > MAX_DECOMPRESSED_REQUEST_BODY_BYTES {
        return Err(());
    }
    Ok(decoded)
}

fn rewrite_turn_metadata(metadata: &mut Value, ids: &ConvergedRequestIds) {
    match metadata {
        Value::Object(object) => {
            object.insert(
                "installation_id".into(),
                Value::String(ids.installation_id.clone()),
            );
            object.insert("session_id".into(), Value::String(ids.session_id.clone()));
            object.insert("thread_id".into(), Value::String(ids.thread_id.clone()));
            object.insert("turn_id".into(), Value::String(ids.turn_id.clone()));
            object.insert("window_id".into(), Value::String(ids.window_id.clone()));
            object.insert(
                "turn_started_at_unix_ms".into(),
                Value::from(ids.turn_started_at_unix_ms),
            );
        }
        Value::String(text) => {
            if let Ok(mut nested) = serde_json::from_str::<Value>(text)
                && nested.is_object()
            {
                rewrite_turn_metadata(&mut nested, ids);
                if let Ok(rendered) = serde_json::to_string(&nested) {
                    *text = rendered;
                }
            }
        }
        _ => {}
    }
}

fn rewrite_turn_metadata_header(value: &HeaderValue, ids: &ConvergedRequestIds) -> HeaderValue {
    let Ok(text) = value.to_str() else {
        return value.clone();
    };
    let Ok(mut metadata) = serde_json::from_str::<Value>(text) else {
        return value.clone();
    };
    if !metadata.is_object() {
        return value.clone();
    }
    rewrite_turn_metadata(&mut metadata, ids);
    serde_json::to_string(&metadata)
        .ok()
        .and_then(|text| HeaderValue::from_str(&text).ok())
        .unwrap_or_else(|| value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_converge_device_and_session_but_keep_threads_and_turns_distinct() {
        let mut first_headers = HeaderMap::new();
        first_headers.insert("session-id", HeaderValue::from_static("client-session-a"));
        let first = converged_request_ids(
            &first_headers,
            "installation-id",
            "converged-session",
            "account-123",
        );
        let again = converged_request_ids(
            &first_headers,
            "installation-id",
            "converged-session",
            "account-123",
        );
        let mut second_headers = HeaderMap::new();
        second_headers.insert("session_id", HeaderValue::from_static("client-session-b"));
        let second = converged_request_ids(
            &second_headers,
            "installation-id",
            "converged-session",
            "account-123",
        );

        assert_eq!(first.installation_id, again.installation_id);
        assert_eq!(first.session_id, again.session_id);
        assert_eq!(first.thread_id, again.thread_id);
        assert_ne!(first.thread_id, second.thread_id);
        assert_ne!(first.turn_id, again.turn_id);
        assert_eq!(
            uuid::Uuid::parse_str(&first.turn_id)
                .unwrap()
                .get_version_num(),
            7
        );
        assert_eq!(first.window_id, format!("{}:0", first.thread_id));
    }

    #[test]
    fn request_ids_fall_back_to_converged_session_when_client_session_is_absent() {
        let first = converged_request_ids(
            &HeaderMap::new(),
            "installation-id",
            "converged-session",
            "account-123",
        );
        let second = converged_request_ids(
            &HeaderMap::new(),
            "installation-id",
            "converged-session",
            "account-123",
        );
        assert_eq!(first.thread_id, second.thread_id);
    }

    #[test]
    fn rewrite_converges_all_documented_header_and_body_carriers() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("session-id", HeaderValue::from_static("client-session"));
        let ids = converged_request_ids(
            &headers,
            "stable-installation",
            "stable-session",
            "account-123",
        );
        let body = Bytes::from(
            r#"{"session_id":"session","thread_id":"thread","turn_id":"turn","client_metadata":{"x-codex-installation-id":"old","other":"keep","x-codex-turn-metadata":"{\"installation_id\":\"old\",\"thread_id\":\"thread\",\"keep\":true}"},"input":"unchanged"}"#,
        );
        let rewritten: Value =
            serde_json::from_slice(&rewrite_converged_request_body(&headers, body, &ids).unwrap())
                .unwrap();
        assert_eq!(rewritten["session_id"], "session");
        assert_eq!(rewritten["thread_id"], "thread");
        assert_eq!(rewritten["turn_id"], "turn");
        assert_eq!(rewritten["input"], "unchanged");
        assert_eq!(rewritten["client_metadata"]["other"], "keep");
        assert_eq!(
            rewritten["client_metadata"]["x-codex-installation-id"],
            ids.installation_id
        );
        assert_eq!(rewritten["client_metadata"]["session_id"], ids.session_id);
        assert_eq!(rewritten["client_metadata"]["thread_id"], ids.thread_id);
        assert_eq!(rewritten["client_metadata"]["turn_id"], ids.turn_id);
        assert_eq!(
            rewritten["client_metadata"]["x-codex-window-id"],
            ids.window_id
        );
        let turn: Value = serde_json::from_str(
            rewritten["client_metadata"]["x-codex-turn-metadata"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(turn["installation_id"], ids.installation_id);
        assert_eq!(turn["session_id"], ids.session_id);
        assert_eq!(turn["thread_id"], ids.thread_id);
        assert_eq!(turn["turn_id"], ids.turn_id);
        assert_eq!(turn["window_id"], ids.window_id);
        assert!(turn["turn_started_at_unix_ms"].is_i64());
        assert_eq!(turn["keep"], true);

        let header =
            rewrite_turn_metadata_header(&HeaderValue::from_static(r#"{"keep":"value"}"#), &ids);
        let metadata: Value = serde_json::from_str(header.to_str().unwrap()).unwrap();
        assert_eq!(metadata["installation_id"], ids.installation_id);
        assert_eq!(metadata["session_id"], ids.session_id);
        assert_eq!(metadata["thread_id"], ids.thread_id);
        assert_eq!(metadata["turn_id"], ids.turn_id);
        assert_eq!(metadata["window_id"], ids.window_id);
        assert_eq!(
            metadata["turn_started_at_unix_ms"],
            ids.turn_started_at_unix_ms
        );
        assert_eq!(metadata["keep"], "value");

        let mut outbound_headers = HeaderMap::new();
        apply_converged_id_headers(&mut outbound_headers, &ids);
        assert_eq!(
            outbound_headers["x-codex-installation-id"],
            ids.installation_id
        );
        assert_eq!(outbound_headers["session-id"], ids.session_id);
        assert_eq!(outbound_headers["session_id"], ids.session_id);
        assert_eq!(outbound_headers["thread-id"], ids.thread_id);
        assert_eq!(outbound_headers["x-client-request-id"], ids.thread_id);
        assert_eq!(outbound_headers["x-codex-window-id"], ids.window_id);
    }

    #[test]
    fn rewrite_creates_metadata_and_preserves_invalid_or_non_object_turn_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let ids = converged_request_ids(
            &headers,
            "stable-installation",
            "stable-session",
            "account-123",
        );
        let rewritten: Value = serde_json::from_slice(
            &rewrite_converged_request_body(
                &headers,
                Bytes::from(
                    r#"{"session_id":"keep","client_metadata":{"x-codex-turn-metadata":"[1]"}}"#,
                ),
                &ids,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(rewritten["session_id"], "keep");
        assert_eq!(
            rewritten["client_metadata"]["x-codex-installation-id"],
            "stable-installation"
        );
        assert_eq!(rewritten["client_metadata"]["x-codex-turn-metadata"], "[1]");
        let header = rewrite_turn_metadata_header(&HeaderValue::from_static("[1]"), &ids);
        assert_eq!(header, HeaderValue::from_static("[1]"));
        let invalid = rewrite_turn_metadata_header(&HeaderValue::from_static("invalid"), &ids);
        assert_eq!(invalid, HeaderValue::from_static("invalid"));
    }

    #[test]
    fn rewrite_handles_zstd_and_preserves_unrelated_top_level_fields() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("content-encoding", HeaderValue::from_static("zstd"));
        let ids = converged_request_ids(
            &headers,
            "stable-installation",
            "stable-session",
            "account-123",
        );
        let input = r#"{"session_id":"session","thread_id":"thread","input":"unchanged","client_metadata":{"x-codex-turn-metadata":"{\"thread_id\":\"thread\"}"}}"#;
        let compressed = zstd::stream::encode_all(input.as_bytes(), 0).unwrap();
        let rewritten =
            rewrite_converged_request_body(&headers, Bytes::from(compressed), &ids).unwrap();
        let decoded = decode_zstd_body(&rewritten).unwrap();
        let value: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(value["session_id"], "session");
        assert_eq!(value["thread_id"], "thread");
        assert_eq!(value["input"], "unchanged");
        assert_eq!(
            value["client_metadata"]["x-codex-installation-id"],
            "stable-installation"
        );
        let turn: Value = serde_json::from_str(
            value["client_metadata"]["x-codex-turn-metadata"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(turn["thread_id"], ids.thread_id);
        assert_eq!(turn["installation_id"], ids.installation_id);
    }

    #[test]
    fn upstream_url_preserves_the_encoded_path_and_query_after_local_token() {
        let uri: axum::http::Uri = format!(
            "/{}/{}/responses%2Fcompact?beta=one%2Ftwo",
            RELAY_PATH_PREFIX, "token"
        )
        .parse()
        .unwrap();
        assert_eq!(
            upstream_url(&uri, "token").as_deref(),
            Some("https://chatgpt.com/backend-api/codex/responses%2Fcompact?beta=one%2Ftwo")
        );
    }
}
