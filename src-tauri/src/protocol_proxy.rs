use crate::models::{
    AppError, CodexChatReasoningConfig, ProviderAccount, ProviderProfile, RouteConsoleSnapshot,
    RouteLogEntry, RouteSettings,
};
use futures_util::StreamExt;
use reqwest::{
    Client,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
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
    sync::{Mutex, Semaphore, oneshot},
};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 8 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 64;
const TOOL_SEARCH_PROXY_NAME: &str = "tool_search";
const CUSTOM_TOOL_INPUT_FIELD: &str = "input";
const CHAT_TOOL_NAME_MAX_LEN: usize = 64;
const CUSTOM_TOOL_INPUT_DESCRIPTION: &str = "Raw string input for the original custom tool. Preserve formatting exactly and follow the original tool definition embedded in the description.";

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexToolKind {
    Function,
    Namespace,
    Custom,
    ToolSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchProxyAction {
    AddFile,
    DeleteFile,
    UpdateFile,
    ReplaceFile,
    Batch,
}

impl PatchProxyAction {
    fn suffix(self) -> &'static str {
        match self {
            Self::AddFile => "add_file",
            Self::DeleteFile => "delete_file",
            Self::UpdateFile => "update_file",
            Self::ReplaceFile => "replace_file",
            Self::Batch => "batch",
        }
    }
}

#[derive(Debug, Clone)]
struct CodexToolSpec {
    kind: CodexToolKind,
    name: String,
    namespace: Option<String>,
    patch_action: Option<PatchProxyAction>,
}

#[derive(Debug, Clone, Default)]
struct CodexToolContext {
    chat_tools: Vec<Value>,
    seen_names: HashSet<String>,
    specs: HashMap<String, CodexToolSpec>,
}

impl CodexToolContext {
    fn add(&mut self, chat_name: String, spec: CodexToolSpec, tool: Value) {
        if chat_name.is_empty() || !self.seen_names.insert(chat_name.clone()) {
            return;
        }
        self.specs.insert(chat_name, spec);
        self.chat_tools.push(tool);
    }

    fn add_function(&mut self, tool: &Value, namespace: Option<&str>) {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            return;
        };
        let chat_name = namespace
            .map(|value| flatten_namespace_name(value, name))
            .unwrap_or_else(|| name.to_string());
        let parameters = tool
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object","properties":{}}));
        let mut function = json!({
            "name":chat_name,
            "description":tool.get("description").cloned().unwrap_or(Value::Null),
            "parameters":parameters
        });
        if let Some(strict) = tool.get("strict") {
            function["strict"] = strict.clone();
        }
        let chat_tool = json!({"type":"function","function":function});
        self.add(
            chat_name,
            CodexToolSpec {
                kind: if namespace.is_some() {
                    CodexToolKind::Namespace
                } else {
                    CodexToolKind::Function
                },
                name: name.to_string(),
                namespace: namespace.map(str::to_owned),
                patch_action: None,
            },
            chat_tool,
        );
    }

    fn add_custom(&mut self, tool: &Value) {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            return;
        };
        if is_apply_patch_tool(tool, name) {
            self.add_patch_tools(name, tool.get("description").and_then(Value::as_str));
            return;
        }
        let description = format!(
            "Original tool definition:\n```json\n{}\n```",
            canonical_json_string(tool)
        );
        self.add(
            name.to_string(),
            CodexToolSpec {
                kind: CodexToolKind::Custom,
                name: name.to_string(),
                namespace: None,
                patch_action: None,
            },
            json!({"type":"function","function":{
                "name":name,
                "description":description,
                "parameters":{"type":"object","properties":{
                    CUSTOM_TOOL_INPUT_FIELD:{"type":"string","description":CUSTOM_TOOL_INPUT_DESCRIPTION}
                },"required":[CUSTOM_TOOL_INPUT_FIELD]}
            }}),
        );
    }

    fn add_patch_tools(&mut self, original_name: &str, description: Option<&str>) {
        for action in [
            PatchProxyAction::AddFile,
            PatchProxyAction::DeleteFile,
            PatchProxyAction::UpdateFile,
            PatchProxyAction::ReplaceFile,
            PatchProxyAction::Batch,
        ] {
            let chat_name = patch_proxy_name(original_name, action);
            self.add(
                chat_name.clone(),
                CodexToolSpec {
                    kind: CodexToolKind::Custom,
                    name: original_name.to_owned(),
                    namespace: None,
                    patch_action: Some(action),
                },
                json!({"type":"function","function":{
                    "name":chat_name,
                    "description":patch_proxy_description(description, action),
                    "parameters":patch_proxy_schema(action)
                }}),
            );
        }
    }

    fn add_tool_search(&mut self) {
        self.add(
            TOOL_SEARCH_PROXY_NAME.into(),
            CodexToolSpec {
                kind: CodexToolKind::ToolSearch,
                name: TOOL_SEARCH_PROXY_NAME.into(),
                namespace: None,
                patch_action: None,
            },
            json!({"type":"function","function":{
                "name":TOOL_SEARCH_PROXY_NAME,
                "description":"Search and load Codex tools, plugins, connectors, and MCP namespaces for the current task.",
                "parameters":{"type":"object","properties":{
                    "query":{"type":"string"},"limit":{"type":"integer"}
                },"required":["query"]}
            }}),
        );
    }

    fn add_response_tool(&mut self, tool: &Value) {
        match tool.get("type").and_then(Value::as_str) {
            Some("function") => self.add_function(tool, None),
            Some("custom") => self.add_custom(tool),
            Some("tool_search") => self.add_tool_search(),
            Some("namespace") => {
                if let (Some(namespace), Some(children)) = (
                    tool.get("name").and_then(Value::as_str),
                    tool.get("tools")
                        .or_else(|| tool.get("children"))
                        .and_then(Value::as_array),
                ) {
                    for child in children {
                        if child.get("type").and_then(Value::as_str) == Some("function") {
                            self.add_function(child, Some(namespace));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn custom_history_call(&self, name: &str, input: &str) -> (String, String) {
        let batch_name = patch_proxy_name(name, PatchProxyAction::Batch);
        if self
            .specs
            .get(&batch_name)
            .is_some_and(|spec| spec.patch_action == Some(PatchProxyAction::Batch))
        {
            return encode_patch_history(name, input);
        }
        (
            name.to_owned(),
            canonical_json_string(&json!({CUSTOM_TOOL_INPUT_FIELD:input})),
        )
    }

    fn custom_choice_name(&self, original_name: &str) -> String {
        let batch = patch_proxy_name(original_name, PatchProxyAction::Batch);
        if self.specs.contains_key(&batch) {
            batch
        } else {
            original_name.to_owned()
        }
    }
}

fn is_apply_patch_tool(tool: &Value, name: &str) -> bool {
    name == "apply_patch"
        || tool
            .pointer("/format/definition")
            .and_then(Value::as_str)
            .is_some_and(|definition| {
                definition.contains("*** Begin Patch")
                    || (definition.contains("begin_patch")
                        && definition.contains("end_patch")
                        && definition.contains("update_hunk"))
            })
}

fn patch_proxy_name(original_name: &str, action: PatchProxyAction) -> String {
    flatten_namespace_name(original_name, action.suffix())
}

fn patch_proxy_description(description: Option<&str>, action: PatchProxyAction) -> String {
    let purpose = match action {
        PatchProxyAction::AddFile => "Create one new file from its path and complete content.",
        PatchProxyAction::DeleteFile => "Delete one file by path.",
        PatchProxyAction::UpdateFile => {
            "Update or move one existing file using ordered line hunks."
        }
        PatchProxyAction::ReplaceFile => "Replace one existing file with complete new content.",
        PatchProxyAction::Batch => {
            "Apply an ordered batch of file additions, deletions, updates, or replacements."
        }
    };
    match description.map(str::trim).filter(|value| !value.is_empty()) {
        Some(description) => format!("{purpose}\n\n{description}"),
        None => purpose.into(),
    }
}

fn patch_hunks_schema() -> Value {
    json!({
        "type":"array",
        "items":{
            "type":"object","additionalProperties":false,
            "properties":{
                "context":{"type":"string"},
                "lines":{"type":"array","items":{
                    "type":"object","additionalProperties":false,
                    "properties":{
                        "op":{"type":"string","enum":["context","add","remove"]},
                        "text":{"type":"string"}
                    },
                    "required":["op","text"]
                }}
            },
            "required":["lines"]
        }
    })
}

fn patch_proxy_schema(action: PatchProxyAction) -> Value {
    let operation = json!({
        "type":"object","additionalProperties":false,
        "properties":{
            "type":{"type":"string","enum":["add_file","delete_file","update_file","replace_file"]},
            "path":{"type":"string"},
            "move_to":{"type":"string"},
            "content":{"type":"string"},
            "hunks":patch_hunks_schema()
        },
        "required":["type","path"]
    });
    match action {
        PatchProxyAction::AddFile | PatchProxyAction::ReplaceFile => json!({
            "type":"object","additionalProperties":false,
            "properties":{"path":{"type":"string"},"content":{"type":"string"}},
            "required":["path","content"]
        }),
        PatchProxyAction::DeleteFile => json!({
            "type":"object","additionalProperties":false,
            "properties":{"path":{"type":"string"}},"required":["path"]
        }),
        PatchProxyAction::UpdateFile => json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "path":{"type":"string"},"move_to":{"type":"string"},"hunks":patch_hunks_schema()
            },
            "required":["path","hunks"]
        }),
        PatchProxyAction::Batch => json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "operations":{"type":"array","items":operation},
                "raw_patch":{"type":"string","description":"Fallback complete patch text when structured operations cannot represent the edit."}
            },
            "required":["operations"]
        }),
    }
}

fn flatten_namespace_name(namespace: &str, name: &str) -> String {
    let full_name = format!("{namespace}__{name}");
    if full_name.len() <= CHAT_TOOL_NAME_MAX_LEN {
        return full_name;
    }
    let hash = Sha256::digest(full_name.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = format!("__{hash}");
    let prefix_len = CHAT_TOOL_NAME_MAX_LEN.saturating_sub(suffix.len());
    let mut prefix = String::new();
    for ch in full_name.chars() {
        if prefix.len() + ch.len_utf8() > prefix_len {
            break;
        }
        prefix.push(ch);
    }
    format!("{prefix}{suffix}")
}

fn build_tool_context(input: &Value) -> CodexToolContext {
    let mut context = CodexToolContext::default();
    for tool in input
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        context.add_response_tool(tool);
    }
    if let Some(history) = input.get("input") {
        collect_tool_search_output_tools(history, &mut context);
    }
    context
}

fn collect_tool_search_output_tools(value: &Value, context: &mut CodexToolContext) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_tool_search_output_tools(item, context);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_search_output")
                && let Some(tools) = object.get("tools").and_then(Value::as_array)
            {
                for tool in tools {
                    context.add_response_tool(tool);
                }
            }
            for child in object.values() {
                collect_tool_search_output_tools(child, context);
            }
        }
        _ => {}
    }
}

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
    client: Client,
    headers: HeaderMap,
    token: String,
    timeout: Duration,
    concurrency: Arc<Semaphore>,
    telemetry: Arc<ProxyTelemetry>,
    provider: ProviderProfile,
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
        settings: &RouteSettings,
    ) -> Result<ProxyEndpoint, AppError> {
        if settings.port != 0 {
            self.stop().await;
        }
        let address = settings.listen_address.trim();
        let bind = format!("{address}:{}", settings.port);
        let listener = TcpListener::bind(&bind)
            .await
            .map_err(|error| AppError::Proxy(format!("无法监听 {bind}：{error}")))?;
        let address = listener
            .local_addr()
            .map_err(|error| AppError::Proxy(error.to_string()))?;
        let token = format!("ct_{}", uuid::Uuid::new_v4().simple());
        let endpoint = ProxyEndpoint {
            base_url: format!("http://{}:{}/v1", settings.listen_address, address.port()),
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
            model: provider.models.first().cloned().unwrap_or_default(),
            started_at: chrono::Utc::now().timestamp(),
            request_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            active_requests: AtomicU64::new(0),
            last_latency_ms: AtomicU64::new(0),
            next_log_id: AtomicU64::new(1),
            logs: Mutex::new(VecDeque::with_capacity(200)),
        });
        let timeout = Duration::from_secs(provider.timeout_secs.max(1));
        let headers = build_upstream_headers(
            account.api_key.as_deref().unwrap_or_default(),
            &provider.headers,
            &account.headers,
        )?;
        let client = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(30)))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|error| AppError::Proxy(error.to_string()))?;
        let config = Arc::new(ProxyConfig {
            upstream_url,
            client,
            headers,
            token,
            timeout,
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            telemetry: telemetry.clone(),
            provider: provider.clone(),
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
                                Ok(status) if status < 400 => {
                                    config.telemetry.success_count.fetch_add(1, Ordering::Relaxed);
                                    (status, None)
                                }
                                Ok(status) => {
                                    config.telemetry.error_count.fetch_add(1, Ordering::Relaxed);
                                    (status, Some("请求失败（敏感详情已隐藏）".into()))
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

    #[cfg(test)]
    async fn endpoint(&self) -> Option<ProxyEndpoint> {
        self.inner
            .lock()
            .await
            .current
            .as_ref()
            .map(|proxy| proxy.endpoint.clone())
    }

    pub async fn console(
        &self,
        settings: RouteSettings,
        page: usize,
        page_size: usize,
    ) -> RouteConsoleSnapshot {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let state = self.inner.lock().await;
        let Some(proxy) = state.current.as_ref() else {
            return RouteConsoleSnapshot {
                settings,
                ..Default::default()
            };
        };
        let telemetry = &proxy.telemetry;
        let logs = telemetry.logs.lock().await;
        let log_total = logs.len();
        let start = (page - 1).saturating_mul(page_size).min(log_total);
        let page_logs = logs
            .iter()
            .rev()
            .skip(start)
            .take(page_size)
            .cloned()
            .collect();
        RouteConsoleSnapshot {
            settings,
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
            logs: page_logs,
            log_total,
            log_page: page,
            log_page_size: page_size,
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

pub(crate) fn build_upstream_headers(
    api_key: &str,
    provider: &Value,
    account: &Value,
) -> Result<HeaderMap, AppError> {
    let mut headers = HeaderMap::new();
    if !api_key.is_empty() {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|_| AppError::InvalidConfig("API Key 包含无效字符".into()))?,
        );
    }
    for values in [provider, account] {
        let Some(values) = values.as_object() else {
            continue;
        };
        for (name, value) in values {
            let (Ok(name), Some(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                value
                    .as_str()
                    .and_then(|value| HeaderValue::from_str(value).ok()),
            ) else {
                continue;
            };
            if name != reqwest::header::HOST && name != reqwest::header::CONTENT_LENGTH {
                headers.insert(name, value);
            }
        }
    }
    Ok(headers)
}

async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    config: Arc<ProxyConfig>,
) -> anyhow::Result<u16> {
    let _permit = match config.concurrency.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            write_json_error(
                &mut stream,
                503,
                "proxy_busy",
                "Too many concurrent requests",
            )
            .await?;
            return Ok(503);
        }
    };
    if !peer.ip().is_loopback() {
        write_json_error(&mut stream, 403, "loopback_only", "Loopback access only").await?;
        return Ok(403);
    }

    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err((status, code, message)) => {
            write_json_error(&mut stream, status, code, &message).await?;
            return Ok(status);
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
        return Ok(404);
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
        return Ok(401);
    }

    let input: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(error) => {
            write_json_error(&mut stream, 400, "invalid_json", &error.to_string()).await?;
            return Ok(400);
        }
    };
    let wants_stream = input
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reasoning = resolve_reasoning_config(&config.provider, &input);
    let (upstream_body, tool_context) =
        match responses_to_chat_with_context(&input, reasoning.as_ref()) {
            Ok(value) => value,
            Err(message) => {
                write_json_error(&mut stream, 400, "invalid_request", &message).await?;
                return Ok(400);
            }
        };

    let upstream = config
        .client
        .post(&config.upstream_url)
        .headers(config.headers.clone())
        .json(&upstream_body);
    let response = match tokio::time::timeout(config.timeout, upstream.send()).await {
        Ok(Ok(response)) => response,
        Err(_) => {
            write_json_error(
                &mut stream,
                504,
                "upstream_timeout",
                "Upstream response timed out",
            )
            .await?;
            return Ok(504);
        }
        Ok(Err(error)) => {
            write_json_error(
                &mut stream,
                502,
                "upstream_unavailable",
                &safe_error(&error.to_string()),
            )
            .await?;
            return Ok(502);
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = sanitize_upstream_error(&body);
        write_json_error(&mut stream, status.as_u16(), "upstream_error", &message).await?;
        return Ok(status.as_u16());
    }

    if wants_stream {
        proxy_stream(&mut stream, response, config.timeout, &tool_context).await?;
    } else {
        let body = match tokio::time::timeout(config.timeout, response.bytes()).await {
            Ok(Ok(body)) if body.len() <= MAX_BODY_BYTES => body,
            Ok(Ok(_)) => {
                write_json_error(
                    &mut stream,
                    502,
                    "upstream_response_too_large",
                    "Upstream response exceeds 8 MiB",
                )
                .await?;
                return Ok(502);
            }
            Ok(Err(error)) => {
                write_json_error(
                    &mut stream,
                    502,
                    "invalid_upstream_response",
                    &error.to_string(),
                )
                .await?;
                return Ok(502);
            }
            Err(_) => {
                write_json_error(
                    &mut stream,
                    504,
                    "upstream_timeout",
                    "Upstream response body timed out",
                )
                .await?;
                return Ok(504);
            }
        };
        let value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) => {
                write_json_error(
                    &mut stream,
                    502,
                    "invalid_upstream_response",
                    &safe_error(&error.to_string()),
                )
                .await?;
                return Ok(502);
            }
        };
        let converted = chat_to_response_with_context(&value, &tool_context);
        write_json(&mut stream, 200, &converted).await?;
    }
    Ok(200)
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

#[cfg(test)]
fn responses_to_chat(input: &Value) -> Result<Value, String> {
    responses_to_chat_with_reasoning(input, None)
}

#[cfg(test)]
fn responses_to_chat_with_reasoning(
    input: &Value,
    reasoning: Option<&CodexChatReasoningConfig>,
) -> Result<Value, String> {
    responses_to_chat_with_context(input, reasoning).map(|(value, _)| value)
}

fn responses_to_chat_with_context(
    input: &Value,
    reasoning: Option<&CodexChatReasoningConfig>,
) -> Result<(Value, CodexToolContext), String> {
    let tool_context = build_tool_context(input);
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
            append_response_history(&mut messages, items, &tool_context);
        }
        Some(item @ Value::Object(_)) => {
            append_response_history(&mut messages, std::slice::from_ref(item), &tool_context);
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
        ("seed", "seed"),
        ("user", "user"),
        ("stream", "stream"),
    ] {
        if let Some(value) = input.get(from) {
            target.insert(to.into(), value.clone());
        }
    }
    if input.get("stream").and_then(Value::as_bool) == Some(true) {
        target.insert("stream_options".into(), json!({"include_usage":true}));
    }
    if !tool_context.chat_tools.is_empty() {
        target.insert(
            "tools".into(),
            Value::Array(tool_context.chat_tools.clone()),
        );
        if let Some(choice) = input.get("tool_choice") {
            target.insert(
                "tool_choice".into(),
                response_tool_choice_to_chat(choice, &tool_context),
            );
        }
        if let Some(parallel) = input.get("parallel_tool_calls") {
            target.insert("parallel_tool_calls".into(), parallel.clone());
        }
    }
    if let Some(format) = input.pointer("/text/format") {
        target.insert("response_format".into(), response_format_to_chat(format));
    }
    apply_reasoning_options(input, &mut output, reasoning);
    Ok((output, tool_context))
}

fn resolve_reasoning_config(
    provider: &ProviderProfile,
    _body: &Value,
) -> Option<CodexChatReasoningConfig> {
    if let Some(mut config) = provider.codex_chat_reasoning.clone() {
        if config.supports_effort == Some(true) && config.supports_thinking.is_none() {
            config.supports_thinking = Some(true);
        }
        return Some(config);
    }
    None
}

#[cfg(test)]
fn reasoning_config(
    thinking: bool,
    effort: bool,
    thinking_param: &str,
    effort_param: &str,
    mode: Option<&str>,
    output: &str,
) -> CodexChatReasoningConfig {
    CodexChatReasoningConfig {
        supports_thinking: Some(thinking),
        supports_effort: Some(effort),
        thinking_param: Some(thinking_param.into()),
        effort_param: Some(effort_param.into()),
        effort_value_mode: mode.map(str::to_owned),
        output_format: Some(output.into()),
    }
}

fn apply_reasoning_options(
    body: &Value,
    result: &mut Value,
    config: Option<&CodexChatReasoningConfig>,
) {
    let Some(config) = config else { return };
    let Some(enabled) = reasoning_requested(body) else {
        return;
    };
    let supports_effort = config.supports_effort.unwrap_or(false);
    if config.supports_thinking.unwrap_or(false) || supports_effort {
        match config.thinking_param.as_deref().unwrap_or("thinking") {
            "thinking" => {
                result["thinking"] = json!({"type": if enabled { "enabled" } else { "disabled" }})
            }
            "enable_thinking" => result["enable_thinking"] = json!(enabled),
            "reasoning_split" => result["reasoning_split"] = json!(enabled),
            _ => {}
        }
    }
    let effort_param = config.effort_param.as_deref().unwrap_or("reasoning_effort");
    if !enabled {
        if effort_param == "reasoning.effort" {
            result["reasoning"] = json!({"effort":"none"});
        }
        return;
    }
    if !supports_effort {
        return;
    }
    let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return;
    };
    let Some(mapped) = map_reasoning_effort(effort, config.effort_value_mode.as_deref()) else {
        return;
    };
    match effort_param {
        "reasoning_effort" => result["reasoning_effort"] = json!(mapped),
        "reasoning.effort" => result["reasoning"] = json!({"effort":mapped}),
        _ => {}
    }
}

fn reasoning_requested(body: &Value) -> Option<bool> {
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
        return Some(!matches!(
            effort.trim().to_ascii_lowercase().as_str(),
            "none" | "off" | "disabled"
        ));
    }
    body.get("reasoning").map(|value| !value.is_null())
}

fn map_reasoning_effort(effort: &str, mode: Option<&str>) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    if matches!(effort.as_str(), "none" | "off" | "disabled") {
        return None;
    }
    match mode.unwrap_or("passthrough") {
        "deepseek" => Some(if matches!(effort.as_str(), "max" | "xhigh") {
            "max"
        } else {
            "high"
        }),
        "low_high" => Some(if matches!(effort.as_str(), "minimal" | "low") {
            "low"
        } else {
            "high"
        }),
        "openrouter" => match effort.as_str() {
            "max" | "xhigh" => Some("xhigh"),
            "high" => Some("high"),
            "medium" => Some("medium"),
            "low" => Some("low"),
            "minimal" => Some("minimal"),
            _ => None,
        },
        _ => match effort.as_str() {
            "minimal" => Some("minimal"),
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" => Some("max"),
            _ => None,
        },
    }
}

fn append_response_history(messages: &mut Vec<Value>, items: &[Value], context: &CodexToolContext) {
    let mut known_calls = HashSet::new();
    for item in items {
        append_response_item(messages, item, context, &mut known_calls);
    }
}

fn append_response_item(
    messages: &mut Vec<Value>,
    item: &Value,
    context: &CodexToolContext,
    known_calls: &mut HashSet<String>,
) {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match kind {
        "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = content_to_text(item.get("output"));
            if !call_id.is_empty() && known_calls.contains(call_id) {
                messages.push(json!({
                    "role":"tool",
                    "tool_call_id":call_id,
                    "content":output
                }));
            } else {
                // Chat Completions rejects tool messages without a preceding
                // assistant tool call. Preserve the information as user text
                // instead of emitting an invalid orphan tool message.
                messages.push(json!({
                    "role":"user",
                    "content":format!("Tool result{}:\n{}",
                        if call_id.is_empty() { String::new() } else { format!(" ({call_id})") },
                        plain_content(&output))
                }));
            }
        }
        "function_call" => {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let chat_name = item
                .get("namespace")
                .and_then(Value::as_str)
                .map(|namespace| flatten_namespace_name(namespace, name))
                .unwrap_or_else(|| name.to_string());
            let call_id = history_call_id(item, &chat_name);
            known_calls.insert(call_id.clone());
            append_assistant_tool_call(
                messages,
                json!({
                    "id":call_id,
                    "type":"function",
                    "function":{
                        "name":chat_name,
                        "arguments":normalize_history_arguments(item.get("arguments"))
                    }
                }),
            );
        }
        "custom_tool_call" => {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let call_id = history_call_id(item, name);
            known_calls.insert(call_id.clone());
            let input = item
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (chat_name, arguments) = context.custom_history_call(name, input);
            append_assistant_tool_call(
                messages,
                json!({
                    "id":call_id,
                    "type":"function",
                    "function":{
                        "name":chat_name,
                        "arguments":arguments
                    }
                }),
            );
        }
        "tool_search_call" if context.specs.contains_key(TOOL_SEARCH_PROXY_NAME) => {
            let call_id = history_call_id(item, TOOL_SEARCH_PROXY_NAME);
            known_calls.insert(call_id.clone());
            append_assistant_tool_call(
                messages,
                json!({
                    "id":call_id,
                    "type":"function",
                    "function":{
                        "name":TOOL_SEARCH_PROXY_NAME,
                        "arguments":normalize_history_arguments(item.get("arguments"))
                    }
                }),
            );
        }
        "reasoning" => {
            if let Some(text) = reasoning_history_text(item) {
                append_assistant_text(messages, &text, "reasoning_content");
            }
        }
        "message" => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let role = if role == "developer" { "system" } else { role };
            let content = content_to_text(item.get("content"));
            if role == "assistant" {
                append_assistant_content(messages, content);
            } else {
                messages.push(json!({"role":role,"content":content}));
            }
        }
        _ if item.get("role").is_some() => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let role = if role == "developer" { "system" } else { role };
            let content = content_to_text(item.get("content"));
            if role == "assistant" {
                append_assistant_content(messages, content);
            } else {
                messages.push(json!({"role":role,"content":content}));
            }
        }
        _ => {}
    }
}

fn append_assistant_content(messages: &mut Vec<Value>, content: Value) {
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("assistant")
    {
        let existing = last.get("content").cloned().unwrap_or(Value::Null);
        last["content"] = merge_chat_content(existing, content);
    } else {
        messages.push(json!({"role":"assistant","content":content}));
    }
}

fn append_assistant_text(messages: &mut Vec<Value>, text: &str, field: &str) {
    if messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("assistant")
    {
        messages.push(json!({"role":"assistant","content":Value::Null}));
    }
    let last = messages.last_mut().expect("assistant message was inserted");
    let current = last.get(field).and_then(Value::as_str).unwrap_or_default();
    last[field] = if current.is_empty() {
        json!(text)
    } else {
        json!(format!("{current}\n{text}"))
    };
}

fn append_assistant_tool_call(messages: &mut Vec<Value>, tool_call: Value) {
    if messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("assistant")
    {
        messages.push(json!({"role":"assistant","content":Value::Null,"tool_calls":[]}));
    }
    let last = messages.last_mut().expect("assistant message was inserted");
    if last.get("tool_calls").and_then(Value::as_array).is_none() {
        last["tool_calls"] = json!([]);
    }
    last["tool_calls"]
        .as_array_mut()
        .expect("tool_calls was initialized as an array")
        .push(tool_call);
}

fn merge_chat_content(left: Value, right: Value) -> Value {
    match (left, right) {
        (Value::Null, right) => right,
        (left, Value::Null) => left,
        (Value::String(left), Value::String(right)) => {
            if left.is_empty() {
                json!(right)
            } else if right.is_empty() {
                json!(left)
            } else {
                json!(format!("{left}\n{right}"))
            }
        }
        (Value::Array(mut left), Value::Array(right)) => {
            left.extend(right);
            Value::Array(left)
        }
        (Value::Array(mut left), right) => {
            left.push(json!({"type":"text","text":plain_content(&right)}));
            Value::Array(left)
        }
        (left, Value::Array(mut right)) => {
            right.insert(0, json!({"type":"text","text":plain_content(&left)}));
            Value::Array(right)
        }
        (left, right) => json!(format!(
            "{}\n{}",
            plain_content(&left),
            plain_content(&right)
        )),
    }
}

fn plain_content(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        value => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn history_call_id(item: &Value, name: &str) -> String {
    if let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        return id.to_owned();
    }
    let digest = Sha256::digest(canonical_json_string(item).as_bytes());
    format!(
        "call_{}_{}",
        name.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(16)
            .collect::<String>(),
        digest
            .iter()
            .take(6)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn normalize_history_arguments(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::Object(value)) => canonical_json_string(&Value::Object(value.clone())),
        Some(Value::String(value)) if value.trim().is_empty() => "{}".into(),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            match serde_json::from_str::<Value>(value) {
                Ok(Value::Object(value)) => canonical_json_string(&Value::Object(value)),
                Ok(value) => canonical_json_string(&json!({"input":value})),
                Err(_) => canonical_json_string(&json!({"input":value})),
            }
        }
        Some(value) if !value.is_null() => canonical_json_string(&json!({"input":value})),
        _ => "{}".into(),
    }
}

fn reasoning_history_text(item: &Value) -> Option<String> {
    let parts = item
        .get("summary")
        .or_else(|| item.get("content"))
        .and_then(Value::as_array)?;
    let text = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn content_to_text(content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => {
            let converted = parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_image" | "image_url") => part
                        .pointer("/image_url/url")
                        .or_else(|| part.get("image_url"))
                        .or_else(|| part.get("url"))
                        .and_then(Value::as_str)
                        .map(|url| json!({"type":"image_url","image_url":{"url":url}})),
                    Some("input_text" | "output_text" | "text") | None => part
                        .as_str()
                        .or_else(|| part.get("text").and_then(Value::as_str))
                        .map(|text| json!({"type":"text","text":text})),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if converted
                .iter()
                .all(|part| part.get("type") == Some(&json!("text")))
            {
                Value::String(
                    converted
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                Value::Array(converted)
            }
        }
        Some(value) => value.clone(),
        None => Value::String(String::new()),
    }
}

fn response_format_to_chat(format: &Value) -> Value {
    match format.get("type").and_then(Value::as_str) {
        Some("json_schema") => json!({
            "type":"json_schema",
            "json_schema": {
                "name": format.get("name").cloned().unwrap_or_else(|| json!("response")),
                "description": format.get("description").cloned().unwrap_or(Value::Null),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
                "strict": format.get("strict").cloned().unwrap_or(json!(false))
            }
        }),
        Some("json_object") => json!({"type":"json_object"}),
        _ => json!({"type":"text"}),
    }
}

fn response_tool_choice_to_chat(choice: &Value, context: &CodexToolContext) -> Value {
    match choice.get("type").and_then(Value::as_str) {
        Some("function") => {
            let name = choice
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let namespace = choice.get("namespace").and_then(Value::as_str);
            let chat_name = namespace
                .map(|value| flatten_namespace_name(value, name))
                .unwrap_or_else(|| name.to_string());
            json!({"type":"function","function":{"name":chat_name}})
        }
        Some("custom") => {
            let original_name = choice
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({"type":"function","function":{
                "name":context.custom_choice_name(original_name)
            }})
        }
        Some("tool_search") if context.specs.contains_key(TOOL_SEARCH_PROXY_NAME) => {
            json!({"type":"function","function":{"name":TOOL_SEARCH_PROXY_NAME}})
        }
        _ => choice.clone(),
    }
}

#[cfg(test)]
fn chat_to_response(chat: &Value) -> Value {
    chat_to_response_with_context(chat, &CodexToolContext::default())
}

fn chat_to_response_with_context(chat: &Value, context: &CodexToolContext) -> Value {
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
            output.push(chat_tool_call_to_response(call, context));
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

fn chat_tool_call_to_response(call: &Value, context: &CodexToolContext) -> Value {
    let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default();
    let chat_name = call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let item_id = response_tool_item_id(call_id, chat_name, context);
    let arguments = call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    match context.specs.get(chat_name) {
        Some(spec) if spec.kind == CodexToolKind::Custom => json!({
            "id":item_id,"type":"custom_tool_call","status":"completed",
            "call_id":call_id,"name":spec.name,"input":custom_tool_input(spec, arguments)
        }),
        Some(spec) if spec.kind == CodexToolKind::ToolSearch => json!({
            "type":"tool_search_call","status":"completed","execution":"client",
            "call_id":call_id,"arguments":parse_arguments_object(arguments)
        }),
        Some(spec) => json!({
            "id":item_id,"type":"function_call","status":"completed","call_id":call_id,
            "name":spec.name,"namespace":spec.namespace,"arguments":arguments
        }),
        None => json!({
            "id":item_id,"type":"function_call","status":"completed","call_id":call_id,
            "name":chat_name,"arguments":arguments
        }),
    }
}

fn response_tool_item_id(call_id: &str, chat_name: &str, context: &CodexToolContext) -> String {
    let prefix = if context
        .specs
        .get(chat_name)
        .is_some_and(|spec| spec.kind == CodexToolKind::Custom)
    {
        "ctc"
    } else {
        "fc"
    };
    format!("{prefix}_{call_id}")
}

fn custom_tool_input(spec: &CodexToolSpec, arguments: &str) -> String {
    if let Some(action) = spec.patch_action {
        return reconstruct_patch_input(action, arguments);
    }
    raw_custom_tool_input(arguments)
}

fn raw_custom_tool_input(arguments: &str) -> String {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get(CUSTOM_TOOL_INPUT_FIELD)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| arguments.to_string())
}

fn encode_patch_history(original_name: &str, input: &str) -> (String, String) {
    let operations = parse_patch_operations(input);
    if operations.len() == 1
        && let Some(action) = operations[0]
            .get("type")
            .and_then(Value::as_str)
            .and_then(patch_action_from_type)
    {
        return (
            patch_proxy_name(original_name, action),
            patch_operation_arguments(&operations[0], action),
        );
    }
    (
        patch_proxy_name(original_name, PatchProxyAction::Batch),
        canonical_json_string(&json!({"operations":operations,"raw_patch":input})),
    )
}

fn reconstruct_patch_input(action: PatchProxyAction, arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_owned();
    };
    if let Some(raw_patch) = value
        .get("raw_patch")
        .or_else(|| value.get("patch"))
        .or_else(|| value.get(CUSTOM_TOOL_INPUT_FIELD))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        return raw_patch.to_owned();
    }
    let operations = match action {
        PatchProxyAction::AddFile => vec![json!({
            "type":"add_file","path":string_field(&value,"path"),
            "content":string_field(&value,"content")
        })],
        PatchProxyAction::DeleteFile => vec![json!({
            "type":"delete_file","path":string_field(&value,"path")
        })],
        PatchProxyAction::UpdateFile => vec![json!({
            "type":"update_file","path":string_field(&value,"path"),
            "move_to":string_field(&value,"move_to"),
            "hunks":value.get("hunks").cloned().unwrap_or_else(|| json!([]))
        })],
        PatchProxyAction::ReplaceFile => vec![json!({
            "type":"replace_file","path":string_field(&value,"path"),
            "content":string_field(&value,"content")
        })],
        PatchProxyAction::Batch => value
            .get("operations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    build_patch_text(&operations)
}

fn string_field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn patch_action_from_type(value: &str) -> Option<PatchProxyAction> {
    match value {
        "add_file" => Some(PatchProxyAction::AddFile),
        "delete_file" => Some(PatchProxyAction::DeleteFile),
        "update_file" => Some(PatchProxyAction::UpdateFile),
        "replace_file" => Some(PatchProxyAction::ReplaceFile),
        _ => None,
    }
}

fn patch_operation_arguments(operation: &Value, action: PatchProxyAction) -> String {
    let value = match action {
        PatchProxyAction::AddFile | PatchProxyAction::ReplaceFile => json!({
            "path":string_field(operation,"path"),"content":string_field(operation,"content")
        }),
        PatchProxyAction::DeleteFile => json!({"path":string_field(operation,"path")}),
        PatchProxyAction::UpdateFile => {
            let mut value = json!({
                "path":string_field(operation,"path"),
                "hunks":operation.get("hunks").cloned().unwrap_or_else(|| json!([]))
            });
            if let Some(move_to) = operation
                .get("move_to")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                value["move_to"] = json!(move_to);
            }
            value
        }
        PatchProxyAction::Batch => json!({"operations":[operation]}),
    };
    canonical_json_string(&value)
}

fn parse_patch_operations(input: &str) -> Vec<Value> {
    let mut operations = Vec::new();
    let mut current: Option<serde_json::Map<String, Value>> = None;
    let mut content = Vec::new();
    let mut hunks = Vec::new();
    let mut current_hunk: Option<serde_json::Map<String, Value>> = None;
    let mut hunk_lines = Vec::new();

    fn flush_hunk(
        current_hunk: &mut Option<serde_json::Map<String, Value>>,
        hunk_lines: &mut Vec<Value>,
        hunks: &mut Vec<Value>,
    ) {
        if let Some(mut hunk) = current_hunk.take() {
            hunk.insert("lines".into(), Value::Array(std::mem::take(hunk_lines)));
            hunks.push(Value::Object(hunk));
        }
    }
    fn flush_operation(
        current: &mut Option<serde_json::Map<String, Value>>,
        content: &mut Vec<String>,
        hunks: &mut Vec<Value>,
        operations: &mut Vec<Value>,
    ) {
        if let Some(mut operation) = current.take() {
            match operation.get("type").and_then(Value::as_str) {
                Some("add_file" | "replace_file") => {
                    operation.insert("content".into(), content.join("\n").into());
                }
                Some("update_file") => {
                    operation.insert("hunks".into(), Value::Array(std::mem::take(hunks)));
                }
                _ => {}
            }
            content.clear();
            operations.push(Value::Object(operation));
        }
    }

    for line in input.lines() {
        if matches!(line, "*** Begin Patch" | "*** End Patch") {
            continue;
        }
        let header = [
            ("*** Add File: ", "add_file"),
            ("*** Delete File: ", "delete_file"),
            ("*** Update File: ", "update_file"),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| line.strip_prefix(prefix).map(|path| (kind, path)));
        if let Some((kind, path)) = header {
            flush_hunk(&mut current_hunk, &mut hunk_lines, &mut hunks);
            flush_operation(&mut current, &mut content, &mut hunks, &mut operations);
            current = Some(serde_json::Map::from_iter([
                ("type".into(), kind.into()),
                ("path".into(), path.into()),
            ]));
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some(operation) = current.as_mut() {
                operation.insert("move_to".into(), path.into());
            }
            continue;
        }
        if let Some(context) = line.strip_prefix("@@") {
            flush_hunk(&mut current_hunk, &mut hunk_lines, &mut hunks);
            current_hunk = Some(serde_json::Map::from_iter([(
                "context".into(),
                context.trim().into(),
            )]));
            continue;
        }
        match current
            .as_ref()
            .and_then(|operation| operation.get("type"))
            .and_then(Value::as_str)
        {
            Some("add_file" | "replace_file") => {
                if let Some(value) = line.strip_prefix('+') {
                    content.push(value.to_owned());
                }
            }
            Some("update_file") => {
                let (op, text) = match line.as_bytes().first() {
                    Some(b'+') => ("add", &line[1..]),
                    Some(b'-') => ("remove", &line[1..]),
                    Some(b' ') => ("context", &line[1..]),
                    _ => ("context", line),
                };
                hunk_lines.push(json!({"op":op,"text":text}));
            }
            _ => {}
        }
    }
    flush_hunk(&mut current_hunk, &mut hunk_lines, &mut hunks);
    flush_operation(&mut current, &mut content, &mut hunks, &mut operations);
    operations
}

fn build_patch_text(operations: &[Value]) -> String {
    let mut output = String::from("*** Begin Patch");
    for operation in operations {
        let path = string_field(operation, "path");
        match operation
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "add_file" => {
                output.push_str(&format!("\n*** Add File: {path}"));
                append_added_content(&mut output, string_field(operation, "content"));
            }
            "delete_file" => output.push_str(&format!("\n*** Delete File: {path}")),
            "update_file" => {
                output.push_str(&format!("\n*** Update File: {path}"));
                if let Some(move_to) = operation
                    .get("move_to")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    output.push_str(&format!("\n*** Move to: {move_to}"));
                }
                for hunk in operation
                    .get("hunks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let context = string_field(hunk, "context");
                    if context.is_empty() {
                        output.push_str("\n@@");
                    } else {
                        output.push_str(&format!("\n@@ {context}"));
                    }
                    for line in hunk
                        .get("lines")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let prefix = match line.get("op").and_then(Value::as_str) {
                            Some("add") => '+',
                            Some("remove" | "delete") => '-',
                            _ => ' ',
                        };
                        output.push('\n');
                        output.push(prefix);
                        output.push_str(string_field(line, "text"));
                    }
                }
            }
            "replace_file" => {
                output.push_str(&format!("\n*** Delete File: {path}"));
                output.push_str(&format!("\n*** Add File: {path}"));
                append_added_content(&mut output, string_field(operation, "content"));
            }
            _ => {}
        }
    }
    output.push_str("\n*** End Patch");
    output
}

fn append_added_content(output: &mut String, content: &str) {
    for line in content.split('\n') {
        output.push_str("\n+");
        output.push_str(line.strip_suffix('\r').unwrap_or(line));
    }
}

fn parse_arguments_object(arguments: &str) -> Value {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({"query":arguments}))
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into()),
                        canonical_json_string(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn canonical_tool_arguments(arguments: &str) -> String {
    if arguments.trim().is_empty() {
        return "{}".into();
    }
    serde_json::from_str::<Value>(arguments)
        .map(|value| canonical_json_string(&value))
        .unwrap_or_else(|_| arguments.to_string())
}

fn completed_tool_input_events(
    item: &Value,
    item_id: &str,
    raw_arguments: &str,
    output_index: usize,
) -> Vec<(&'static str, Value)> {
    if item.get("type").and_then(Value::as_str) == Some("custom_tool_call") {
        let input = item
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut events = Vec::with_capacity(2);
        if !input.is_empty() {
            events.push((
                "response.custom_tool_call_input.delta",
                json!({
                    "type":"response.custom_tool_call_input.delta",
                    "item_id":item_id,
                    "output_index":output_index,
                    "delta":input
                }),
            ));
        }
        events.push((
            "response.custom_tool_call_input.done",
            json!({
                "type":"response.custom_tool_call_input.done",
                "item_id":item_id,
                "output_index":output_index,
                "input":input
            }),
        ));
        events
    } else {
        vec![(
            "response.function_call_arguments.done",
            json!({
                "type":"response.function_call_arguments.done",
                "item_id":item_id,
                "output_index":output_index,
                "arguments":canonical_tool_arguments(raw_arguments)
            }),
        )]
    }
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
    let cached = usage
        .and_then(|value| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .or_else(|| value.get("cached_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning = usage
        .and_then(|value| {
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .or_else(|| value.get("reasoning_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({"input_tokens":input,"input_tokens_details":{"cached_tokens":cached},"output_tokens":output,"output_tokens_details":{"reasoning_tokens":reasoning},"total_tokens":total})
}

async fn proxy_stream(
    stream: &mut TcpStream,
    response: reqwest::Response,
    idle_timeout: Duration,
    tool_context: &CodexToolContext,
) -> anyhow::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nX-Content-Type-Options: nosniff\r\n\r\n").await?;
    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let mut sequence = 0_u64;
    let mut full_text = String::new();
    let mut usage = map_usage(None);
    let mut model = String::new();
    let mut tool_calls: BTreeMap<u64, Value> = BTreeMap::new();
    let mut tool_output_indices = HashMap::new();
    let mut text_output_index = None;
    let mut next_output_index = 0_usize;
    send_sse(stream, "response.created", &json!({"type":"response.created","sequence_number":sequence,"response":stream_response(&response_id,"in_progress",&model,vec![],usage.clone())})).await?;
    sequence += 1;

    let mut pending = Vec::new();
    let mut bytes = response.bytes_stream();
    loop {
        let Some(chunk) = tokio::time::timeout(idle_timeout, bytes.next())
            .await
            .map_err(|_| anyhow::anyhow!("upstream stream idle timeout"))?
        else {
            break;
        };
        let chunk = chunk?;
        pending.extend_from_slice(&chunk);
        if pending.len() > MAX_BODY_BYTES {
            anyhow::bail!("upstream SSE event exceeds 8 MiB");
        }
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
                    let output_index = match text_output_index {
                        Some(index) => index,
                        None => {
                            let index = next_output_index;
                            next_output_index += 1;
                            text_output_index = Some(index);
                            send_sse(stream, "response.output_item.added", &json!({"type":"response.output_item.added","sequence_number":sequence,"output_index":index,"item":{"id":message_id,"type":"message","status":"in_progress","role":"assistant","content":[]}})).await?;
                            sequence += 1;
                            send_sse(stream, "response.content_part.added", &json!({"type":"response.content_part.added","sequence_number":sequence,"output_index":index,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})).await?;
                            sequence += 1;
                            index
                        }
                    };
                    full_text.push_str(text);
                    send_sse(stream, "response.output_text.delta", &json!({"type":"response.output_text.delta","sequence_number":sequence,"output_index":output_index,"content_index":0,"delta":text})).await?;
                    sequence += 1;
                }
                if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                        merge_tool_call_deltas(&mut tool_calls, std::slice::from_ref(call));
                        let Some(merged) = tool_calls.get_mut(&index) else {
                            continue;
                        };
                        if merged
                            .get("id")
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty)
                            && merged
                                .pointer("/function/name")
                                .and_then(Value::as_str)
                                .is_some_and(|name| !name.is_empty())
                        {
                            merged["id"] = json!(format!("call_{index}"));
                        }
                        let call_id = merged.get("id").and_then(Value::as_str).unwrap_or_default();
                        let name = merged
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let first = !call_id.is_empty()
                            && !name.is_empty()
                            && !tool_output_indices.contains_key(&index);
                        if first {
                            let output_index = next_output_index;
                            next_output_index += 1;
                            tool_output_indices.insert(index, output_index);
                            let mut item = chat_tool_call_to_response(merged, tool_context);
                            item["status"] = json!("in_progress");
                            match item.get("type").and_then(Value::as_str) {
                                Some("custom_tool_call") => item["input"] = json!(""),
                                Some("function_call") => item["arguments"] = json!(""),
                                Some("tool_search_call") => item["arguments"] = json!({}),
                                _ => {}
                            }
                            send_sse(stream, "response.output_item.added", &json!({"type":"response.output_item.added","sequence_number":sequence,"output_index":output_index,"item":item})).await?;
                            sequence += 1;
                        }
                        if let Some(&output_index) = tool_output_indices.get(&index) {
                            let custom = tool_context
                                .specs
                                .get(name)
                                .is_some_and(|spec| spec.kind == CodexToolKind::Custom);
                            if !custom {
                                let arguments = if first {
                                    merged
                                        .pointer("/function/arguments")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                } else {
                                    call.pointer("/function/arguments")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                };
                                if arguments.is_empty() {
                                    continue;
                                }
                                let event = "response.function_call_arguments.delta";
                                let item_id = response_tool_item_id(call_id, name, tool_context);
                                send_sse(stream, event, &json!({"type":event,"sequence_number":sequence,"item_id":item_id,"output_index":output_index,"delta":arguments})).await?;
                                sequence += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut indexed_output = Vec::new();
    if let Some(output_index) = text_output_index {
        send_sse(stream, "response.output_text.done", &json!({"type":"response.output_text.done","sequence_number":sequence,"output_index":output_index,"content_index":0,"text":full_text})).await?;
        sequence += 1;
        send_sse(stream, "response.content_part.done", &json!({"type":"response.content_part.done","sequence_number":sequence,"output_index":output_index,"content_index":0,"part":{"type":"output_text","text":full_text,"annotations":[]}})).await?;
        sequence += 1;
        let message = json!({"id":message_id,"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":full_text,"annotations":[]}]});
        send_sse(stream, "response.output_item.done", &json!({"type":"response.output_item.done","sequence_number":sequence,"output_index":output_index,"item":message})).await?;
        sequence += 1;
        indexed_output.push((output_index, message));
    }

    for (tool_index, mut call) in tool_calls {
        if call
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            call["id"] = json!(format!("call_{tool_index}"));
        }
        let chat_name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if chat_name.is_empty() {
            continue;
        }
        let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default();
        let item_id = response_tool_item_id(call_id, chat_name, tool_context);
        let raw_arguments = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let item = chat_tool_call_to_response(&call, tool_context);
        let output_index = match tool_output_indices.get(&tool_index).copied() {
            Some(index) => index,
            None => {
                let index = next_output_index;
                next_output_index += 1;
                let mut pending_item = item.clone();
                pending_item["status"] = json!("in_progress");
                match pending_item.get("type").and_then(Value::as_str) {
                    Some("custom_tool_call") => pending_item["input"] = json!(""),
                    Some("function_call") => pending_item["arguments"] = json!(""),
                    Some("tool_search_call") => pending_item["arguments"] = json!({}),
                    _ => {}
                }
                send_sse(stream, "response.output_item.added", &json!({"type":"response.output_item.added","sequence_number":sequence,"output_index":index,"item":pending_item})).await?;
                sequence += 1;
                index
            }
        };
        for (event, mut payload) in
            completed_tool_input_events(&item, &item_id, raw_arguments, output_index)
        {
            payload["sequence_number"] = json!(sequence);
            send_sse(stream, event, &payload).await?;
            sequence += 1;
        }
        send_sse(stream, "response.output_item.done", &json!({"type":"response.output_item.done","sequence_number":sequence,"output_index":output_index,"item":item})).await?;
        sequence += 1;
        indexed_output.push((output_index, item));
    }
    indexed_output.sort_by_key(|(index, _)| *index);
    let output = indexed_output.into_iter().map(|(_, item)| item).collect();
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
        let entry = target.entry(index).or_insert_with(|| {
            json!({
                "id":"",
                "type":"function",
                "function":{"name":"","arguments":""}
            })
        });
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
    let prefix = format!("{:X}\r\n", data.len());
    let mut frame = Vec::with_capacity(prefix.len() + data.len() + 2);
    frame.extend_from_slice(prefix.as_bytes());
    frame.extend_from_slice(data);
    frame.extend_from_slice(b"\r\n");
    stream.write_all(&frame).await?;
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
        503 => "Service Unavailable",
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
            models: vec![],
            model_metadata: vec![],
            codex_chat_reasoning: None,
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

    fn route_settings() -> RouteSettings {
        RouteSettings::default()
    }

    #[tokio::test]
    async fn aborted_prepare_keeps_current_proxy_running() {
        let manager = ProxyManager::default();
        let first = manager
            .prepare(&provider(), &account(), &route_settings())
            .await
            .unwrap();
        manager.commit().await.unwrap();
        let second = manager
            .prepare(&provider(), &account(), &route_settings())
            .await
            .unwrap();
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
        let first = manager
            .prepare(&provider(), &account(), &route_settings())
            .await
            .unwrap();
        manager.commit().await.unwrap();
        let second = manager
            .prepare(&provider(), &account(), &route_settings())
            .await
            .unwrap();
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
    fn preserves_custom_tool_search_and_namespace_tools_for_chat_upstream() {
        let input = json!({
            "model":"test-model",
            "input":[{"role":"user","content":"change Windows theme"}],
            "tools":[
                {"type":"custom","name":"exec","description":"Run computer control code"},
                {"type":"tool_search"},
                {"type":"namespace","name":"computer_use","tools":[
                    {"type":"function","name":"click","description":"Click UI","parameters":{"type":"object","properties":{"x":{"type":"integer"}}}}
                ]}
            ]
        });
        let (output, context) = responses_to_chat_with_context(&input, None).unwrap();
        let names = output["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["exec", "tool_search", "computer_use__click"]);

        let custom = chat_to_response_with_context(
            &json!({"choices":[{"message":{"tool_calls":[{"id":"call-1","type":"function","function":{"name":"exec","arguments":"{\"input\":\"raw code\"}"}}]}}]}),
            &context,
        );
        assert_eq!(custom["output"][0]["type"], "custom_tool_call");
        assert_eq!(custom["output"][0]["input"], "raw code");

        let search = chat_to_response_with_context(
            &json!({"choices":[{"message":{"tool_calls":[{"id":"call-2","type":"function","function":{"name":"tool_search","arguments":"{\"query\":\"computer\"}"}}]}}]}),
            &context,
        );
        assert_eq!(search["output"][0]["type"], "tool_search_call");
        assert_eq!(search["output"][0]["arguments"]["query"], "computer");

        let namespace = chat_to_response_with_context(
            &json!({"choices":[{"message":{"tool_calls":[{"id":"call-3","type":"function","function":{"name":"computer_use__click","arguments":"{\"x\":10}"}}]}}]}),
            &context,
        );
        assert_eq!(namespace["output"][0]["type"], "function_call");
        assert_eq!(namespace["output"][0]["name"], "click");
        assert_eq!(namespace["output"][0]["namespace"], "computer_use");
    }

    #[test]
    fn exposes_apply_patch_as_structured_chat_functions() {
        let input = json!({
            "model":"test-model",
            "input":"edit files",
            "tools":[{"type":"custom","name":"apply_patch","description":"Patch workspace files"}],
            "tool_choice":{"type":"custom","name":"apply_patch"}
        });
        let (output, context) = responses_to_chat_with_context(&input, None).unwrap();
        let names = output["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool.pointer("/function/name").unwrap().as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "apply_patch__add_file",
                "apply_patch__delete_file",
                "apply_patch__update_file",
                "apply_patch__replace_file",
                "apply_patch__batch"
            ]
        );
        assert_eq!(
            output["tool_choice"]["function"]["name"],
            "apply_patch__batch"
        );
        assert_eq!(
            output["tools"][2]["function"]["parameters"]["properties"]["hunks"]["items"]["properties"]
                ["lines"]["items"]["required"],
            json!(["op", "text"])
        );
        assert!(
            context
                .specs
                .values()
                .all(|spec| spec.name == "apply_patch" && spec.patch_action.is_some())
        );
    }

    #[test]
    fn replays_and_reconstructs_apply_patch_calls_without_json_wrapper() {
        let patch = "*** Begin Patch\n*** Add File: src/new.rs\n+fn main() {}\n*** End Patch";
        let input = json!({
            "model":"test-model",
            "input":[{"type":"custom_tool_call","call_id":"patch-1","name":"apply_patch","input":patch}],
            "tools":[{"type":"custom","name":"apply_patch"}]
        });
        let (output, context) = responses_to_chat_with_context(&input, None).unwrap();
        let call = &output["messages"][0]["tool_calls"][0];
        assert_eq!(call["function"]["name"], "apply_patch__add_file");
        let arguments = call["function"]["arguments"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(arguments).unwrap(),
            json!({"path":"src/new.rs","content":"fn main() {}"})
        );

        let response = chat_to_response_with_context(
            &json!({"choices":[{"message":{"tool_calls":[{
                "id":"patch-2","type":"function","function":{
                    "name":"apply_patch__add_file","arguments":arguments
                }
            }]}}]}),
            &context,
        );
        assert_eq!(response["output"][0]["type"], "custom_tool_call");
        assert_eq!(response["output"][0]["name"], "apply_patch");
        assert_eq!(response["output"][0]["input"], patch);
    }

    #[test]
    fn reconstructs_update_batch_and_invalid_patch_arguments() {
        let request = json!({
            "model":"test-model","input":"edit",
            "tools":[{"type":"custom","name":"apply_patch"}]
        });
        let (_, context) = responses_to_chat_with_context(&request, None).unwrap();
        let update = chat_tool_call_to_response(
            &json!({"id":"u","function":{
                "name":"apply_patch__update_file",
                "arguments":canonical_json_string(&json!({
                    "path":"src/lib.rs","move_to":"src/core.rs",
                    "hunks":[{"context":"fn old()","lines":[
                        {"op":"remove","text":"fn old() {}"},
                        {"op":"add","text":"fn new() {}"}
                    ]}]
                }))
            }}),
            &context,
        );
        assert_eq!(
            update["input"],
            "*** Begin Patch\n*** Update File: src/lib.rs\n*** Move to: src/core.rs\n@@ fn old()\n-fn old() {}\n+fn new() {}\n*** End Patch"
        );

        let batch = chat_tool_call_to_response(
            &json!({"id":"b","function":{
                "name":"apply_patch__batch",
                "arguments":canonical_json_string(&json!({"operations":[
                    {"type":"delete_file","path":"old.txt"},
                    {"type":"replace_file","path":"new.txt","content":"one\ntwo"}
                ]}))
            }}),
            &context,
        );
        assert_eq!(
            batch["input"],
            "*** Begin Patch\n*** Delete File: old.txt\n*** Delete File: new.txt\n*** Add File: new.txt\n+one\n+two\n*** End Patch"
        );

        let invalid = chat_tool_call_to_response(
            &json!({"id":"x","function":{
                "name":"apply_patch__batch","arguments":"not-json"
            }}),
            &context,
        );
        assert_eq!(invalid["input"], "not-json");
    }

    #[test]
    fn completes_structured_patch_stream_as_raw_custom_input() {
        let request = json!({
            "model":"test-model","input":"edit",
            "tools":[{"type":"custom","name":"apply_patch"}]
        });
        let (_, context) = responses_to_chat_with_context(&request, None).unwrap();
        let arguments = r#"{"content":"hello","path":"new.txt"}"#;
        let call = json!({"id":"patch-stream","function":{
            "name":"apply_patch__add_file","arguments":arguments
        }});
        let item = chat_tool_call_to_response(&call, &context);
        let id = response_tool_item_id("patch-stream", "apply_patch__add_file", &context);
        let events = completed_tool_input_events(&item, &id, arguments, 0);
        assert_eq!(item["type"], "custom_tool_call");
        assert_eq!(item["name"], "apply_patch");
        assert_eq!(id, "ctc_patch-stream");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1["delta"], item["input"]);
        assert_eq!(events[1].1["input"], item["input"]);
        assert!(!events[0].1["delta"].as_str().unwrap().starts_with('{'));
    }

    #[test]
    fn replays_namespace_tool_history_with_the_upstream_chat_name() {
        let input = json!({
            "model":"test-model",
            "input":[
                {"type":"function_call","call_id":"call-1","namespace":"computer_use","name":"click","arguments":"{\"x\":10}"},
                {"type":"function_call_output","call_id":"call-1","output":"ok"}
            ],
            "tools":[{"type":"namespace","name":"computer_use","tools":[
                {"type":"function","name":"click","parameters":{"type":"object"}}
            ]}]
        });
        let (output, _) = responses_to_chat_with_context(&input, None).unwrap();
        assert_eq!(
            output["messages"][0]["tool_calls"][0]["function"]["name"],
            "computer_use__click"
        );
        assert_eq!(output["messages"][1]["tool_call_id"], "call-1");
    }

    #[test]
    fn merges_contiguous_assistant_history_and_parallel_tool_calls() {
        let input = json!({
            "model":"test-model",
            "input":[
                {"role":"user","content":"change the theme"},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"I should use tools"}]},
                {"role":"assistant","content":[{"type":"output_text","text":"I will do that."}]},
                {"type":"function_call","call_id":"call-1","name":"read","arguments":"{\"path\":\"a\"}"},
                {"type":"custom_tool_call","call_id":"call-2","name":"computer","input":"open Settings"},
                {"type":"function_call_output","call_id":"call-1","output":"ok"},
                {"type":"custom_tool_call_output","call_id":"call-2","output":"done"}
            ],
            "tools":[
                {"type":"function","name":"read","parameters":{"type":"object"}},
                {"type":"custom","name":"computer","description":"Control the computer"}
            ]
        });
        let output = responses_to_chat(&input).unwrap();
        let assistant = &output["messages"][1];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"], "I will do that.");
        assert_eq!(assistant["reasoning_content"], "I should use tools");
        assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(output["messages"][2]["tool_call_id"], "call-1");
        assert_eq!(output["messages"][3]["tool_call_id"], "call-2");
    }

    #[test]
    fn orphan_tool_results_become_user_context_instead_of_invalid_tool_messages() {
        let output = responses_to_chat(&json!({
            "model":"test-model",
            "input":[{"type":"function_call_output","call_id":"missing","output":"result"}]
        }))
        .unwrap();
        assert_eq!(output["messages"][0]["role"], "user");
        assert!(
            output["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("result")
        );
    }

    #[test]
    fn normalizes_empty_and_invalid_history_arguments() {
        let input = json!({
            "model":"test-model",
            "input":[
                {"type":"function_call","call_id":"empty","name":"read","arguments":""},
                {"type":"function_call","call_id":"invalid","name":"write","arguments":"raw text"}
            ],
            "tools":[
                {"type":"function","name":"read","parameters":{"type":"object"}},
                {"type":"function","name":"write","parameters":{"type":"object"}}
            ]
        });
        let output = responses_to_chat(&input).unwrap();
        let calls = output["messages"][0]["tool_calls"].as_array().unwrap();
        assert_eq!(calls[0]["function"]["arguments"], "{}");
        assert_eq!(calls[1]["function"]["arguments"], r#"{"input":"raw text"}"#);
    }

    #[test]
    fn omits_tool_controls_when_no_chat_tools_exist() {
        let output = responses_to_chat(&json!({
            "model":"test-model",
            "input":"hello",
            "tool_choice":"required",
            "parallel_tool_calls":true
        }))
        .unwrap();
        assert!(output.get("tool_choice").is_none());
        assert!(output.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn decodes_only_complete_custom_tool_input_without_json_wrapper() {
        assert_eq!(
            raw_custom_tool_input(r#"{"input":"open Settings"}"#),
            "open Settings"
        );
        assert_eq!(
            raw_custom_tool_input(r#"{"input":"open \"Settings\"\nthen click"}"#),
            "open \"Settings\"\nthen click"
        );
        assert_eq!(raw_custom_tool_input(r#"{"input":"深色模式"}"#), "深色模式");
        let incomplete = r#"{"input":"incomplete\"#;
        assert_eq!(raw_custom_tool_input(incomplete), incomplete);
    }

    #[test]
    fn completes_custom_tool_stream_with_raw_input_and_custom_item_id() {
        let input = json!({
            "model":"test-model",
            "input":"change Windows theme",
            "tools":[{"type":"custom","name":"exec","description":"Run computer control code"}]
        });
        let (_, context) = responses_to_chat_with_context(&input, None).unwrap();
        let call = json!({
            "id":"call_custom",
            "type":"function",
            "function":{"name":"exec","arguments":"{\"input\":\"open Settings\"}"}
        });
        let item = chat_tool_call_to_response(&call, &context);
        let item_id = response_tool_item_id("call_custom", "exec", &context);
        let events =
            completed_tool_input_events(&item, &item_id, r#"{"input":"open Settings"}"#, 1);

        assert_eq!(item_id, "ctc_call_custom");
        assert_eq!(item["id"], item_id);
        assert_eq!(item["input"], "open Settings");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "response.custom_tool_call_input.delta");
        assert_eq!(events[0].1["item_id"], "ctc_call_custom");
        assert_eq!(events[0].1["delta"], "open Settings");
        assert_eq!(events[1].0, "response.custom_tool_call_input.done");
        assert_eq!(events[1].1["input"], "open Settings");
        assert!(
            events
                .iter()
                .all(|(event, _)| !event.contains("function_call_arguments"))
        );
        assert!(
            events
                .iter()
                .all(|(_, payload)| !payload.to_string().contains(r#"{\"input\":"#))
        );
    }

    #[test]
    fn completes_function_and_tool_search_with_stable_nonempty_item_ids() {
        let function_call = json!({
            "id":"call_function",
            "type":"function",
            "function":{"name":"read","arguments":"{ \"b\": 2, \"a\": 1 }"}
        });
        let function_context = CodexToolContext::default();
        let function_item = chat_tool_call_to_response(&function_call, &function_context);
        let function_id = response_tool_item_id("call_function", "read", &function_context);
        let function_events =
            completed_tool_input_events(&function_item, &function_id, r#"{ "b": 2, "a": 1 }"#, 1);
        assert_eq!(function_id, "fc_call_function");
        assert_eq!(function_events.len(), 1);
        assert_eq!(function_events[0].1["arguments"], r#"{"a":1,"b":2}"#);

        let request = json!({
            "model":"test-model",
            "input":"find a tool",
            "tools":[{"type":"tool_search"}]
        });
        let (_, search_context) = responses_to_chat_with_context(&request, None).unwrap();
        let search_call = json!({
            "id":"call_search",
            "type":"function",
            "function":{"name":"tool_search","arguments":"{\"query\":\"computer\"}"}
        });
        let search_item = chat_tool_call_to_response(&search_call, &search_context);
        let search_id = response_tool_item_id("call_search", "tool_search", &search_context);
        let search_events =
            completed_tool_input_events(&search_item, &search_id, r#"{"query":"computer"}"#, 1);
        assert_eq!(search_item["type"], "tool_search_call");
        assert_eq!(search_id, "fc_call_search");
        assert_eq!(search_events.len(), 1);
        assert_eq!(search_events[0].1["item_id"], "fc_call_search");
        assert_eq!(search_events[0].1["arguments"], r#"{"query":"computer"}"#);
    }

    #[test]
    fn long_namespace_tool_names_are_stable_bounded_and_collision_resistant() {
        let namespace = "computer_use_namespace_with_a_very_long_provider_identifier";
        let first = flatten_namespace_name(namespace, "control_windows_settings_primary");
        let second = flatten_namespace_name(namespace, "control_windows_settings_secondary");
        assert!(first.len() <= CHAT_TOOL_NAME_MAX_LEN);
        assert!(second.len() <= CHAT_TOOL_NAME_MAX_LEN);
        assert_eq!(
            first,
            flatten_namespace_name(namespace, "control_windows_settings_primary")
        );
        assert_ne!(first, second);
    }

    #[test]
    fn converts_reasoning_structured_output_and_images() {
        let reasoning = reasoning_config(
            true,
            true,
            "thinking",
            "reasoning_effort",
            Some("standard"),
            "reasoning_content",
        );
        let output = responses_to_chat_with_reasoning(&json!({
            "model":"test-model",
            "input":[{"role":"user","content":[
                {"type":"input_text","text":"describe"},
                {"type":"input_image","image_url":"data:image/png;base64,AA=="}
            ]}],
            "reasoning":{"effort":"high"},
            "text":{"format":{"type":"json_schema","name":"answer","schema":{"type":"object"},"strict":true}}
        }), Some(&reasoning)).unwrap();
        assert_eq!(output["reasoning_effort"], "high");
        assert_eq!(output["response_format"]["type"], "json_schema");
        assert_eq!(output["response_format"]["json_schema"]["name"], "answer");
        assert_eq!(output["messages"][0]["content"][0]["type"], "text");
        assert_eq!(output["messages"][0]["content"][1]["type"], "image_url");
    }

    #[test]
    fn only_maps_reasoning_when_provider_has_explicit_configuration() {
        let mut provider = provider();
        provider.name = "Provider name must not trigger guessing".into();
        provider.base_url = "https://example.test/openrouter/deepseek/qwen".into();
        let input =
            json!({"model":"deepseek-reasoner","input":"hello","reasoning":{"effort":"max"}});
        assert!(resolve_reasoning_config(&provider, &input).is_none());

        provider.codex_chat_reasoning = Some(reasoning_config(
            false,
            true,
            "none",
            "reasoning.effort",
            Some("openrouter"),
            "auto",
        ));
        let config = resolve_reasoning_config(&provider, &input).unwrap();
        let output = responses_to_chat_with_reasoning(&input, Some(&config)).unwrap();
        assert_eq!(output["reasoning"]["effort"], "xhigh");
        assert!(output.get("thinking").is_none());
    }

    #[test]
    fn maps_explicit_thinking_and_effort() {
        let mut provider = provider();
        provider.codex_chat_reasoning = Some(reasoning_config(
            true,
            true,
            "thinking",
            "reasoning_effort",
            Some("deepseek"),
            "reasoning_content",
        ));
        let input = json!({
            "model":"deepseek-reasoner",
            "input":"hello",
            "reasoning":{"effort":"xhigh"}
        });
        let config = resolve_reasoning_config(&provider, &input).unwrap();
        let output = responses_to_chat_with_reasoning(&input, Some(&config)).unwrap();
        assert_eq!(output["thinking"]["type"], "enabled");
        assert_eq!(output["reasoning_effort"], "max");
    }

    #[test]
    fn maps_explicit_boolean_thinking_switches() {
        let mut qwen = provider();
        qwen.codex_chat_reasoning = Some(reasoning_config(
            true,
            false,
            "enable_thinking",
            "none",
            None,
            "reasoning_content",
        ));
        let input = json!({"model":"qwen3-coder","input":"hello","reasoning":{"effort":"high"}});
        let config = resolve_reasoning_config(&qwen, &input).unwrap();
        let output = responses_to_chat_with_reasoning(&input, Some(&config)).unwrap();
        assert_eq!(output["enable_thinking"], true);

        let mut kimi = provider();
        kimi.codex_chat_reasoning = Some(reasoning_config(
            true,
            false,
            "thinking",
            "none",
            None,
            "reasoning_content",
        ));
        let input = json!({
            "model":"kimi-k2-thinking",
            "input":"hello",
            "reasoning":{"effort":"high"}
        });
        let config = resolve_reasoning_config(&kimi, &input).unwrap();
        let output = responses_to_chat_with_reasoning(&input, Some(&config)).unwrap();
        assert_eq!(output["thinking"]["type"], "enabled");
    }

    #[test]
    fn custom_headers_override_auth_without_forwarding_transport_headers() {
        let headers = build_upstream_headers(
            "default-key",
            &json!({"x-provider":"one","authorization":"Bearer provider"}),
            &json!({"x-provider":"two","authorization":"Bearer account","host":"bad"}),
        )
        .unwrap();
        assert_eq!(headers.get("x-provider").unwrap(), "two");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer account");
        assert!(headers.get("host").is_none());
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
    fn preserves_cached_and_reasoning_usage_details() {
        let usage = map_usage(Some(&json!({
            "prompt_tokens":20,
            "completion_tokens":8,
            "total_tokens":28,
            "prompt_tokens_details":{"cached_tokens":7},
            "completion_tokens_details":{"reasoning_tokens":5}
        })));
        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 7);
        assert_eq!(usage["output_tokens_details"]["reasoning_tokens"], 5);
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
        let item_id = chat_tool_call_to_response(&calls[&0], &CodexToolContext::default())["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(item_id.starts_with("fc_"));
        assert_eq!(
            chat_tool_call_to_response(&calls[&0], &CodexToolContext::default())["id"],
            item_id
        );
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
