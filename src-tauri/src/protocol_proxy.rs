use crate::models::{
    AppError, ProviderAccount, ProviderProfile, RouteConsoleSnapshot, RouteLogEntry,
};
use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, oneshot},
};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct ProxyEndpoint {
    pub base_url: String,
    pub token: String,
}

struct RunningProxy {
    endpoint: ProxyEndpoint,
    shutdown: oneshot::Sender<()>,
    telemetry: Arc<ProxyTelemetry>,
}

#[derive(Default)]
struct ProxyState {
    current: Option<RunningProxy>,
    pending: Option<RunningProxy>,
}

#[derive(Default)]
pub struct ProxyManager {
    inner: Mutex<ProxyState>,
    activation: Mutex<()>,
}

#[derive(Clone)]
struct ProxyConfig {
    upstream_url: String,
    api_key: String,
    headers: Value,
    token: String,
    timeout: Duration,
    telemetry: Arc<ProxyTelemetry>,
}

struct ProxyTelemetry {
    upstream_url: String,
    provider_name: String,
    account_name: String,
    model: String,
    started_at: i64,
    request_count: AtomicU64,
    success_count: AtomicU64,
    error_count: AtomicU64,
    active_requests: AtomicU64,
    last_latency_ms: AtomicU64,
    next_log_id: AtomicU64,
    logs: Mutex<VecDeque<RouteLogEntry>>,
}

impl ProxyManager {
    pub async fn activation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.activation.lock().await
    }

    pub async fn prepare(
        &self,
        provider: &ProviderProfile,
        account: &ProviderAccount,
    ) -> Result<ProxyEndpoint, AppError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| AppError::Proxy(error.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|error| AppError::Proxy(error.to_string()))?;
        let token = format!("ct_{}", uuid::Uuid::new_v4().simple());
        let endpoint = ProxyEndpoint {
            base_url: format!("http://127.0.0.1:{}/v1", address.port()),
            token: token.clone(),
        };
        let upstream_url = format!(
            "{}/chat/completions",
            provider.base_url.trim_end_matches('/')
        );
        let telemetry = Arc::new(ProxyTelemetry {
            upstream_url: upstream_url.clone(),
            provider_name: provider.name.clone(),
            account_name: account.name.clone(),
            model: provider.default_model.clone(),
            started_at: chrono::Utc::now().timestamp(),
            request_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            active_requests: AtomicU64::new(0),
            last_latency_ms: AtomicU64::new(0),
            next_log_id: AtomicU64::new(1),
            logs: Mutex::new(VecDeque::with_capacity(200)),
        });
        let config = Arc::new(ProxyConfig {
            upstream_url,
            api_key: account.api_key.clone().unwrap_or_default(),
            headers: merge_headers(&provider.headers, &account.headers),
            token,
            timeout: Duration::from_secs(provider.timeout_secs.max(1)),
            telemetry: telemetry.clone(),
        });
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { break };
                        let config = config.clone();
                        tokio::spawn(async move {
                            let started = std::time::Instant::now();
                            config.telemetry.request_count.fetch_add(1, Ordering::Relaxed);
                            config.telemetry.active_requests.fetch_add(1, Ordering::Relaxed);
                            let result = handle_connection(stream, peer, config.clone()).await;
                            config.telemetry.active_requests.fetch_sub(1, Ordering::Relaxed);
                            let latency = started.elapsed().as_millis() as u64;
                            config.telemetry.last_latency_ms.store(latency, Ordering::Relaxed);
                            let (status, message) = match result {
                                Ok(()) => {
                                    config.telemetry.success_count.fetch_add(1, Ordering::Relaxed);
                                    (200, None)
                                }
                                Err(_) => {
                                    config.telemetry.error_count.fetch_add(1, Ordering::Relaxed);
                                    (500, Some("代理内部错误（敏感详情已隐藏）".into()))
                                }
                            };
                            let id = config.telemetry.next_log_id.fetch_add(1, Ordering::Relaxed);
                            let mut logs = config.telemetry.logs.lock().await;
                            if logs.len() >= 200 { logs.pop_front(); }
                            logs.push_back(RouteLogEntry {
                                id,
                                timestamp: chrono::Utc::now().timestamp(),
                                method: "POST".into(),
                                path: "/v1/responses".into(),
                                status,
                                latency_ms: latency,
                                message,
                            });
                        });
                    }
                }
            }
        });

        let mut state = self.inner.lock().await;
        if let Some(previous_pending) = state.pending.take() {
            let _ = previous_pending.shutdown.send(());
        }
        state.pending = Some(RunningProxy {
            endpoint: endpoint.clone(),
            shutdown: shutdown_tx,
            telemetry,
        });
        Ok(endpoint)
    }

    pub async fn commit(&self) -> Result<(), AppError> {
        let mut state = self.inner.lock().await;
        let next = state
            .pending
            .take()
            .ok_or_else(|| AppError::Proxy("no pending proxy activation".into()))?;
        if let Some(previous) = state.current.replace(next) {
            let _ = previous.shutdown.send(());
        }
        Ok(())
    }

    pub async fn abort(&self) {
        if let Some(proxy) = self.inner.lock().await.pending.take() {
            let _ = proxy.shutdown.send(());
        }
    }

    pub async fn stop(&self) {
        let mut state = self.inner.lock().await;
        if let Some(proxy) = state.pending.take() {
            let _ = proxy.shutdown.send(());
        }
        if let Some(proxy) = state.current.take() {
            let _ = proxy.shutdown.send(());
        }
    }

    pub async fn endpoint(&self) -> Option<ProxyEndpoint> {
        self.inner
            .lock()
            .await
            .current
            .as_ref()
            .map(|proxy| proxy.endpoint.clone())
    }

    pub async fn console(&self) -> RouteConsoleSnapshot {
        let state = self.inner.lock().await;
        let Some(proxy) = state.current.as_ref() else {
            return RouteConsoleSnapshot::default();
        };
        let telemetry = &proxy.telemetry;
        RouteConsoleSnapshot {
            running: true,
            base_url: Some(proxy.endpoint.base_url.clone()),
            upstream_url: Some(telemetry.upstream_url.clone()),
            provider_name: Some(telemetry.provider_name.clone()),
            account_name: Some(telemetry.account_name.clone()),
            model: Some(telemetry.model.clone()),
            started_at: Some(telemetry.started_at),
            request_count: telemetry.request_count.load(Ordering::Relaxed),
            success_count: telemetry.success_count.load(Ordering::Relaxed),
            error_count: telemetry.error_count.load(Ordering::Relaxed),
            active_requests: telemetry.active_requests.load(Ordering::Relaxed),
            last_latency_ms: match telemetry.last_latency_ms.load(Ordering::Relaxed) {
                0 => None,
                value => Some(value),
            },
            logs: telemetry.logs.lock().await.iter().cloned().collect(),
        }
    }

    pub async fn clear_logs(&self) {
        let telemetry = self
            .inner
            .lock()
            .await
            .current
            .as_ref()
            .map(|proxy| proxy.telemetry.clone());
        if let Some(telemetry) = telemetry {
            telemetry.logs.lock().await.clear();
        }
    }
}

fn merge_headers(provider: &Value, account: &Value) -> Value {
    let mut headers = provider.as_object().cloned().unwrap_or_default();
    if let Some(values) = account.as_object() {
        headers.extend(values.clone());
    }
    Value::Object(headers)
}

async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    config: Arc<ProxyConfig>,
) -> anyhow::Result<()> {
    if !peer.ip().is_loopback() {
        write_json_error(&mut stream, 403, "loopback_only", "Loopback access only").await?;
        return Ok(());
    }

    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err((status, code, message)) => {
            write_json_error(&mut stream, status, code, &message).await?;
            return Ok(());
        }
    };
    if request.method != "POST" || !matches!(request.path.as_str(), "/v1/responses" | "/responses")
    {
        write_json_error(
            &mut stream,
            404,
            "not_found",
            "Only POST /v1/responses is supported",
        )
        .await?;
        return Ok(());
    }
    let expected = format!("Bearer {}", config.token);
    if request.headers.get("authorization") != Some(&expected) {
        write_json_error(
            &mut stream,
            401,
            "invalid_loopback_token",
            "Invalid proxy token",
        )
        .await?;
        return Ok(());
    }

    let input: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(error) => {
            write_json_error(&mut stream, 400, "invalid_json", &error.to_string()).await?;
            return Ok(());
        }
    };
    let wants_stream = input
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let upstream_body = match responses_to_chat(&input) {
        Ok(value) => value,
        Err(message) => {
            write_json_error(&mut stream, 400, "invalid_request", &message).await?;
            return Ok(());
        }
    };

    let client = reqwest::Client::builder().timeout(config.timeout).build()?;
    let mut upstream = client
        .post(&config.upstream_url)
        .bearer_auth(&config.api_key)
        .json(&upstream_body);
    if let Some(headers) = config.headers.as_object() {
        for (name, value) in headers {
            let (Ok(name), Some(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                value
                    .as_str()
                    .and_then(|value| HeaderValue::from_str(value).ok()),
            ) else {
                continue;
            };
            if name != reqwest::header::AUTHORIZATION && name != reqwest::header::HOST {
                upstream = upstream.header(name, value);
            }
        }
    }
    let response = match upstream.send().await {
        Ok(response) => response,
        Err(error) => {
            write_json_error(
                &mut stream,
                502,
                "upstream_unavailable",
                &safe_error(&error.to_string()),
            )
            .await?;
            return Ok(());
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = sanitize_upstream_error(&body);
        write_json_error(&mut stream, status.as_u16(), "upstream_error", &message).await?;
        return Ok(());
    }

    if wants_stream {
        proxy_stream(&mut stream, response).await?;
    } else {
        let value: Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                write_json_error(
                    &mut stream,
                    502,
                    "invalid_upstream_response",
                    &error.to_string(),
                )
                .await?;
                return Ok(());
            }
        };
        let converted = chat_to_response(&value);
        write_json(&mut stream, 200, &converted).await?;
    }
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, (u16, &'static str, String)> {
    let mut buffer = Vec::with_capacity(4096);
    let header_end = loop {
        if buffer.len() >= MAX_HEADER_BYTES {
            return Err((
                431,
                "headers_too_large",
                "Request headers are too large".into(),
            ));
        }
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| (400, "read_error", error.to_string()))?;
        if read == 0 {
            return Err((
                400,
                "incomplete_request",
                "Connection closed before request completed".into(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_bytes(&buffer, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header_text = std::str::from_utf8(&buffer[..header_end])
        .map_err(|error| (400, "invalid_headers", error.to_string()))?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Err((
            411,
            "content_length_required",
            "Chunked request bodies are not supported".into(),
        ));
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err((
            413,
            "request_too_large",
            "Request body exceeds 8 MiB".into(),
        ));
    }
    while buffer.len() < header_end + content_length {
        let remaining = header_end + content_length - buffer.len();
        let mut chunk = vec![0_u8; remaining.min(64 * 1024)];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| (400, "read_error", error.to_string()))?;
        if read == 0 {
            return Err((
                400,
                "incomplete_request",
                "Connection closed before body completed".into(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: buffer[header_end..header_end + content_length].to_vec(),
    })
}

fn responses_to_chat(input: &Value) -> Result<Value, String> {
    let model = input
        .get("model")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "model is required".to_string())?;
    let mut messages = Vec::new();
    if let Some(instructions) = input
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        messages.push(json!({"role":"system","content":instructions}));
    }
    match input.get("input") {
        Some(Value::String(text)) => messages.push(json!({"role":"user","content":text})),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(message) = response_item_to_chat(item) {
                    messages.push(message);
                }
            }
        }
        Some(item @ Value::Object(_)) => {
            if let Some(message) = response_item_to_chat(item) {
                messages.push(message);
            }
        }
        _ => return Err("input is required".into()),
    }
    if messages.is_empty() {
        return Err("input did not contain a supported message".into());
    }
    let mut output = json!({"model":model,"messages":messages});
    let target = output.as_object_mut().expect("object");
    for (from, to) in [
        ("max_output_tokens", "max_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("parallel_tool_calls", "parallel_tool_calls"),
        ("stream", "stream"),
    ] {
        if let Some(value) = input.get(from) {
            target.insert(to.into(), value.clone());
        }
    }
    if input.get("stream").and_then(Value::as_bool) == Some(true) {
        target.insert("stream_options".into(), json!({"include_usage":true}));
    }
    if let Some(tools) = input.get("tools").and_then(Value::as_array) {
        target.insert(
            "tools".into(),
            Value::Array(tools.iter().filter_map(response_tool_to_chat).collect()),
        );
    }
    if let Some(choice) = input.get("tool_choice") {
        target.insert("tool_choice".into(), response_tool_choice_to_chat(choice));
    }
    Ok(output)
}

fn response_item_to_chat(item: &Value) -> Option<Value> {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match kind {
        "function_call_output" => Some(json!({
            "role":"tool",
            "tool_call_id":item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
            "content":item.get("output").cloned().unwrap_or(Value::String(String::new()))
        })),
        "function_call" => Some(json!({
            "role":"assistant",
            "content":Value::Null,
            "tool_calls":[{"id":item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or(Value::String(String::new())),"type":"function","function":{"name":item.get("name").cloned().unwrap_or(Value::String(String::new())),"arguments":item.get("arguments").cloned().unwrap_or(Value::String("{}".into()))}}]
        })),
        "message" => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let role = if role == "developer" { "system" } else { role };
            Some(json!({"role":role,"content":content_to_text(item.get("content"))}))
        }
        _ if item.get("role").is_some() => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let role = if role == "developer" { "system" } else { role };
            Some(json!({"role":role,"content":content_to_text(item.get("content"))}))
        }
        _ => None,
    }
}

fn content_to_text(content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => Value::String(
            parts
                .iter()
                .filter_map(|part| {
                    part.as_str()
                        .map(str::to_owned)
                        .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_owned))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Some(value) => value.clone(),
        None => Value::String(String::new()),
    }
}

fn response_tool_to_chat(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    let mut function = serde_json::Map::new();
    for key in ["name", "description", "parameters", "strict"] {
        if let Some(value) = tool.get(key) {
            function.insert(key.into(), value.clone());
        }
    }
    Some(json!({"type":"function","function":function}))
}

fn response_tool_choice_to_chat(choice: &Value) -> Value {
    if choice.get("type").and_then(Value::as_str) == Some("function") {
        json!({"type":"function","function":{"name":choice.get("name").cloned().unwrap_or(Value::String(String::new()))}})
    } else {
        choice.clone()
    }
}

fn chat_to_response(chat: &Value) -> Value {
    let response_id = response_id(chat);
    let message = chat
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut output = Vec::new();
    if let Some(text) = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        output.push(json!({"id":format!("msg_{}", uuid::Uuid::new_v4().simple()),"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":text,"annotations":[]}]}));
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            output.push(chat_tool_call_to_response(call));
        }
    }
    json!({
        "id":response_id,
        "object":"response",
        "created_at":chat.get("created").cloned().unwrap_or_else(|| json!(chrono::Utc::now().timestamp())),
        "status":"completed",
        "error":Value::Null,
        "incomplete_details":Value::Null,
        "model":chat.get("model").cloned().unwrap_or(Value::String(String::new())),
        "output":output,
        "usage":map_usage(chat.get("usage")),
    })
}

fn chat_tool_call_to_response(call: &Value) -> Value {
    json!({
        "id":format!("fc_{}", uuid::Uuid::new_v4().simple()),
        "type":"function_call",
        "status":"completed",
        "call_id":call.get("id").cloned().unwrap_or(Value::String(String::new())),
        "name":call.pointer("/function/name").cloned().unwrap_or(Value::String(String::new())),
        "arguments":call.pointer("/function/arguments").cloned().unwrap_or(Value::String("{}".into()))
    })
}

fn map_usage(usage: Option<&Value>) -> Value {
    let input = usage
        .and_then(|v| v.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|v| v.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .and_then(|v| v.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(input + output);
    json!({"input_tokens":input,"input_tokens_details":{"cached_tokens":0},"output_tokens":output,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":total})
}

async fn proxy_stream(stream: &mut TcpStream, response: reqwest::Response) -> anyhow::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nX-Content-Type-Options: nosniff\r\n\r\n").await?;
    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let mut sequence = 0_u64;
    let mut full_text = String::new();
    let mut usage = map_usage(None);
    let mut model = String::new();
    let mut tool_calls: BTreeMap<u64, Value> = BTreeMap::new();
    send_sse(stream, "response.created", &json!({"type":"response.created","sequence_number":sequence,"response":stream_response(&response_id,"in_progress",&model,vec![],usage.clone())})).await?;
    sequence += 1;
    send_sse(stream, "response.output_item.added", &json!({"type":"response.output_item.added","sequence_number":sequence,"output_index":0,"item":{"id":message_id,"type":"message","status":"in_progress","role":"assistant","content":[]}})).await?;
    sequence += 1;
    send_sse(stream, "response.content_part.added", &json!({"type":"response.content_part.added","sequence_number":sequence,"output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})).await?;
    sequence += 1;

    let mut pending = Vec::new();
    let mut bytes = response.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        let chunk = chunk?;
        pending.extend_from_slice(&chunk);
        while let Some((position, separator)) = find_sse_separator(&pending) {
            let block = String::from_utf8_lossy(&pending[..position]).into_owned();
            pending.drain(..position + separator);
            for line in block
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("data:"))
            {
                let data = line.trim_start_matches("data:").trim();
                if data == "[DONE]" {
                    continue;
                }
                let Ok(chunk): Result<Value, _> = serde_json::from_str(data) else {
                    continue;
                };
                if model.is_empty() {
                    model = chunk
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                if chunk.get("usage").is_some_and(|v| !v.is_null()) {
                    usage = map_usage(chunk.get("usage"));
                }
                let Some(delta) = chunk.pointer("/choices/0/delta") else {
                    continue;
                };
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    full_text.push_str(text);
                    send_sse(stream, "response.output_text.delta", &json!({"type":"response.output_text.delta","sequence_number":sequence,"output_index":0,"content_index":0,"delta":text})).await?;
                    sequence += 1;
                }
                if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    merge_tool_call_deltas(&mut tool_calls, calls);
                }
            }
        }
    }

    send_sse(stream, "response.output_text.done", &json!({"type":"response.output_text.done","sequence_number":sequence,"output_index":0,"content_index":0,"text":full_text})).await?;
    sequence += 1;
    send_sse(stream, "response.content_part.done", &json!({"type":"response.content_part.done","sequence_number":sequence,"output_index":0,"content_index":0,"part":{"type":"output_text","text":full_text,"annotations":[]}})).await?;
    sequence += 1;
    let message = json!({"id":message_id,"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":full_text,"annotations":[]}]});
    send_sse(stream, "response.output_item.done", &json!({"type":"response.output_item.done","sequence_number":sequence,"output_index":0,"item":message})).await?;
    sequence += 1;

    let mut output = vec![message];
    for (_, call) in tool_calls {
        let item = chat_tool_call_to_response(&call);
        let index = output.len();
        send_sse(stream, "response.output_item.added", &json!({"type":"response.output_item.added","sequence_number":sequence,"output_index":index,"item":item})).await?;
        sequence += 1;
        send_sse(stream, "response.output_item.done", &json!({"type":"response.output_item.done","sequence_number":sequence,"output_index":index,"item":item})).await?;
        sequence += 1;
        output.push(item);
    }
    let completed = stream_response(&response_id, "completed", &model, output, usage);
    send_sse(
        stream,
        "response.completed",
        &json!({"type":"response.completed","sequence_number":sequence,"response":completed}),
    )
    .await?;
    finish_chunks(stream).await?;
    Ok(())
}

fn merge_tool_call_deltas(target: &mut BTreeMap<u64, Value>, calls: &[Value]) {
    for call in calls {
        let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
        let entry = target.entry(index).or_insert_with(
            || json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
        );
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            entry["id"] = json!(id);
        }
        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
            entry["function"]["name"] = json!(name);
        }
        if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
            let mut combined = entry
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            combined.push_str(arguments);
            entry["function"]["arguments"] = json!(combined);
        }
    }
}

fn stream_response(id: &str, status: &str, model: &str, output: Vec<Value>, usage: Value) -> Value {
    json!({"id":id,"object":"response","created_at":chrono::Utc::now().timestamp(),"status":status,"error":Value::Null,"incomplete_details":Value::Null,"model":model,"output":output,"usage":usage})
}

fn response_id(value: &Value) -> String {
    value
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("resp_{id}"))
        .unwrap_or_else(|| format!("resp_{}", uuid::Uuid::new_v4().simple()))
}

async fn send_sse(stream: &mut TcpStream, event: &str, value: &Value) -> anyhow::Result<()> {
    let data = format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(value)?
    );
    write_chunk(stream, data.as_bytes()).await
}

async fn write_chunk(stream: &mut TcpStream, data: &[u8]) -> anyhow::Result<()> {
    stream
        .write_all(format!("{:X}\r\n", data.len()).as_bytes())
        .await?;
    stream.write_all(data).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn finish_chunks(stream: &mut TcpStream) -> anyhow::Result<()> {
    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn write_json(stream: &mut TcpStream, status: u16, value: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_vec(value)?;
    let reason = status_reason(status);
    stream.write_all(format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n", body.len()).as_bytes()).await?;
    stream.write_all(&body).await?;
    Ok(())
}

async fn write_json_error(
    stream: &mut TcpStream,
    status: u16,
    code: &str,
    message: &str,
) -> anyhow::Result<()> {
    write_json(
        stream,
        status,
        &json!({"error":{"message":message,"type":"proxy_error","code":code}}),
    )
    .await
}

fn sanitize_upstream_error(body: &str) -> String {
    let truncated: String = body.chars().take(MAX_ERROR_BYTES).collect();
    if let Ok(value) = serde_json::from_str::<Value>(&truncated)
        && let Some(message) = value.pointer("/error/message").and_then(Value::as_str)
    {
        return safe_error(message);
    }
    safe_error(&truncated)
}

fn safe_error(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("api key") || lower.contains("authorization") || lower.contains("bearer ") {
        "Upstream request failed; sensitive details were redacted".into()
    } else {
        value.chars().take(MAX_ERROR_BYTES).collect()
    }
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Upstream Response",
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_sse_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = find_bytes(buffer, b"\n\n").map(|position| (position, 2));
    let crlf = find_bytes(buffer, b"\r\n\r\n").map(|position| (position, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> ProviderProfile {
        ProviderProfile {
            id: "provider-1".into(),
            name: "Test".into(),
            protocol: crate::models::ProviderProtocol::ChatCompletions,
            base_url: "http://127.0.0.1:9/v1".into(),
            default_model: "test-model".into(),
            models: vec![],
            headers: json!({}),
            timeout_secs: 1,
            context_window: None,
            auto_compact_threshold: None,
            enabled: true,
            active: false,
            active_account_id: None,
            account_count: 1,
        }
    }

    fn account() -> ProviderAccount {
        ProviderAccount {
            id: "account-1".into(),
            provider_id: Some("provider-1".into()),
            name: "Default".into(),
            auth_kind: crate::models::AccountAuthKind::ApiKey,
            api_key: Some("secret".into()),
            auth_json: None,
            headers: json!({}),
            active: false,
            email: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn aborted_prepare_keeps_current_proxy_running() {
        let manager = ProxyManager::default();
        let first = manager.prepare(&provider(), &account()).await.unwrap();
        manager.commit().await.unwrap();
        let second = manager.prepare(&provider(), &account()).await.unwrap();
        assert_ne!(first.base_url, second.base_url);
        manager.abort().await;
        assert_eq!(manager.endpoint().await.unwrap().base_url, first.base_url);
        let response = reqwest::Client::new()
            .post(format!("{}/responses", first.base_url))
            .bearer_auth(&first.token)
            .json(&json!({"model":"test-model","input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
        manager.stop().await;
    }

    #[tokio::test]
    async fn committing_prepare_replaces_current_proxy() {
        let manager = ProxyManager::default();
        let first = manager.prepare(&provider(), &account()).await.unwrap();
        manager.commit().await.unwrap();
        let second = manager.prepare(&provider(), &account()).await.unwrap();
        manager.commit().await.unwrap();
        assert_eq!(manager.endpoint().await.unwrap().base_url, second.base_url);
        assert_ne!(first.base_url, second.base_url);
        manager.stop().await;
    }

    #[test]
    fn converts_responses_messages_and_tools() {
        let input = json!({
            "model":"test-model",
            "instructions":"Be concise",
            "input":[
                {"role":"user","content":[{"type":"input_text","text":"hello"}]},
                {"type":"function_call","call_id":"call_1","name":"read","arguments":"{\"path\":\"a\"}"},
                {"type":"function_call_output","call_id":"call_1","output":"ok"}
            ],
            "tools":[{"type":"function","name":"read","description":"Read","parameters":{"type":"object"}}],
            "max_output_tokens":123,
            "stream":true
        });
        let output = responses_to_chat(&input).unwrap();
        assert_eq!(output["max_tokens"], 123);
        assert_eq!(output["messages"][0]["role"], "system");
        assert_eq!(output["messages"][1]["content"], "hello");
        assert_eq!(output["messages"][3]["role"], "tool");
        assert_eq!(output["tools"][0]["function"]["name"], "read");
        assert_eq!(output["stream_options"]["include_usage"], true);
    }

    #[test]
    fn converts_non_streaming_chat_response() {
        let chat = json!({
            "id":"chatcmpl_1","model":"test-model","created":1,
            "choices":[{"message":{"role":"assistant","content":"done","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]}}],
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
        });
        let response = chat_to_response(&chat);
        assert_eq!(response["object"], "response");
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"][0]["content"][0]["text"], "done");
        assert_eq!(response["output"][1]["type"], "function_call");
        assert_eq!(response["usage"]["total_tokens"], 5);
    }

    #[test]
    fn merges_streamed_tool_arguments() {
        let mut calls = BTreeMap::new();
        merge_tool_call_deltas(
            &mut calls,
            &[json!({"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"pa"}})],
        );
        merge_tool_call_deltas(
            &mut calls,
            &[json!({"index":0,"function":{"arguments":"th\":\"a\"}"}})],
        );
        assert_eq!(calls[&0]["function"]["arguments"], "{\"path\":\"a\"}");
    }

    #[test]
    fn finds_lf_and_crlf_sse_boundaries() {
        assert_eq!(find_sse_separator(b"data: one\n\ndata: two"), Some((9, 2)));
        assert_eq!(
            find_sse_separator(b"data: one\r\n\r\ndata: two"),
            Some((9, 4))
        );
    }
}
