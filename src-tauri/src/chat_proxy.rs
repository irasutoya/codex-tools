//! 本机 Chat Completions API → OpenAI Responses API 转换代理。
//!
//! 部分第三方服务只提供 Chat Completions API（DeepSeek、Moonshot、GLM、
//! Qwen 等），而 Codex 只讲 Responses API。本模块为这类服务启动一个只监听
//! `127.0.0.1` 的转换代理：Codex 仍然使用 Responses API 请求本机代理，
//! 代理把请求翻译成 Chat Completions 请求转发给上游服务，再把上游的流式 /
//! 非流式响应翻译回 Responses API 格式。
//!
//! 转换规则参考了社区成熟的实现（codex_deepseek_proxy、api2codex、
//! openai-responses-to-chat 等）：`input`/`instructions` → `messages`，
//! `function_call`/`function_call_output` → `tool_calls`/`tool` 消息，
//! `tools` 扁平结构 → Chat 嵌套结构，`max_output_tokens` → `max_tokens`，
//! `reasoning.effort` → `reasoning_effort`；流式输出按
//! `response.output_item.added` / `response.output_text.delta` /
//! `response.function_call_arguments.delta` / `response.completed` 事件还原。

use crate::{
    models::{AppError, ProviderApiType, ProviderProfile},
    network::ClientCache,
};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Json, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, oneshot};

/// 本机转换代理的监听地址（只允许本机访问）。
pub(crate) const PROXY_HOST: &str = "127.0.0.1";
/// 本机转换代理的固定端口。端口保持固定，Codex 配置里的地址跨重启也始终有效。
pub(crate) const PROXY_PORT: u16 = 27777;
/// 转换代理接受的请求体上限。Codex 会把完整会话历史（含 base64 图片）发给
/// 代理，axum 默认只允许 2MB，必须放宽，否则长对话/带图请求会被 413 拒绝。
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
/// 写入 Codex auth.json 的固定占位 Key。真实的服务商 Key 只保存在本应用，
/// 由转换代理注入上游请求，Codex 侧始终使用这个固定 Key。
pub(crate) const PROXY_FIXED_API_KEY: &str = "codex-tools";
/// 写入 Codex 配置的本机代理地址路径。
pub(crate) const PROXY_BASE_PATH: &str = "/v1";

/// 本机转换代理的完整根地址。
pub(crate) fn proxy_base_url() -> String {
    format!("http://{PROXY_HOST}:{PROXY_PORT}{PROXY_BASE_PATH}")
}

// ---------------------------------------------------------------------------
// 注册表：一个共享的本机转换代理
// ---------------------------------------------------------------------------
//
// 只存在一个监听 `127.0.0.1:27777` 的代理。上游指向当前激活的 Chat 类型服务：
// 切换服务时只原地替换共享配置，Codex 缓存的地址永远不变。

#[derive(Default)]
pub(crate) struct ChatProxyRegistry {
    single: Mutex<Option<RunningProxy>>,
}

struct RunningProxy {
    /// 本机监听端口（固定为 PROXY_PORT）。
    port: u16,
    /// 当前配置对应的服务指纹；变化时原地替换上游参数。
    fingerprint: String,
    /// 当前配置所属的服务 ID；删除该服务时清空配置，避免转发到错误的上游。
    owner: String,
    shutdown: oneshot::Sender<()>,
    /// 共享的上游配置。
    config: Arc<ProxyConfigSlot>,
}

impl ChatProxyRegistry {
    /// 确保转换代理正在运行，并把上游配置切换到 `provider`。
    /// 监听地址固定为 `http://127.0.0.1:27777/v1`。
    pub(crate) async fn ensure(&self, provider: &ProviderProfile) -> Result<u16, AppError> {
        let fingerprint = proxy_fingerprint(provider);
        let config = ProxyConfig::from_provider(provider);
        let mut single = self.single.lock().await;
        if let Some(running) = single.as_mut() {
            if running.fingerprint != fingerprint {
                *running.config.current.write().await = config;
                running.fingerprint = fingerprint;
                running.owner = provider.id.clone();
            }
            return Ok(running.port);
        }
        let (port, shutdown, config_slot) = start_proxy(config).await?;
        *single = Some(RunningProxy {
            port,
            fingerprint,
            owner: provider.id.clone(),
            shutdown,
            config: config_slot,
        });
        Ok(port)
    }

    /// 服务被删除时调用：如果它正是当前代理配置的服务，清空上游配置，
    /// 让后续请求返回明确的错误而不是转发到错误的上游。监听保持运行。
    pub(crate) async fn stop(&self, provider_id: &str) {
        let mut single = self.single.lock().await;
        if let Some(running) = single.as_mut()
            && running.owner == provider_id
        {
            *running.config.current.write().await = ProxyConfig::disabled();
            running.fingerprint.clear();
            running.owner.clear();
        }
    }

    /// 停止全部代理（应用退出时调用）。
    pub(crate) async fn stop_all(&self) {
        if let Some(running) = self.single.lock().await.take() {
            let _ = running.shutdown.send(());
        }
    }
}

fn proxy_fingerprint(provider: &ProviderProfile) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        provider.base_url,
        provider.api_key.as_deref().unwrap_or_default(),
        serde_json::to_string(&provider.headers).unwrap_or_default(),
        provider.timeout_secs
    )
}

// ---------------------------------------------------------------------------
// 代理服务
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ProxyConfig {
    upstream_base: String,
    api_key: String,
    headers: Vec<(String, String)>,
}

impl ProxyConfig {
    fn from_provider(provider: &ProviderProfile) -> Self {
        Self {
            upstream_base: provider.base_url.trim_end_matches('/').to_owned(),
            api_key: provider.api_key.clone().unwrap_or_default(),
            headers: provider.headers.clone().into_iter().collect(),
        }
    }

    /// 空配置：所属服务被删除后使用，任何请求都会得到明确的错误提示。
    fn disabled() -> Self {
        Self {
            upstream_base: String::new(),
            api_key: String::new(),
            headers: Vec::new(),
        }
    }

    fn is_disabled(&self) -> bool {
        self.upstream_base.is_empty()
    }
}

/// 共享配置槽：切换/编辑服务时在端口不变的前提下原地替换上游参数。
struct ProxyConfigSlot {
    current: tokio::sync::RwLock<ProxyConfig>,
}

impl ProxyConfigSlot {
    fn new(config: ProxyConfig) -> Arc<Self> {
        Arc::new(Self {
            current: tokio::sync::RwLock::new(config),
        })
    }
}

struct ProxyState {
    config: Arc<ProxyConfigSlot>,
    client: ProxyClient,
    /// 上一轮响应产生的 `reasoning_content` 按输出条目 id 保存，
    /// 供下一轮请求回传（DeepSeek 等要求 thinking 模式的 reasoning_content 必须回传）。
    reasoning_store: Arc<std::sync::Mutex<ReasoningStore>>,
}

/// 有界的 reasoning_content 存储：条目 id → 思考内容。
/// 只保留最近若干轮，超限时淘汰最早的条目。
#[derive(Default)]
struct ReasoningStore {
    entries: HashMap<String, String>,
    order: VecDeque<String>,
}

impl ReasoningStore {
    const MAX_ENTRIES: usize = 2000;

    fn insert(&mut self, id: &str, content: &str) {
        if content.trim().is_empty() {
            return;
        }
        if self.entries.contains_key(id) {
            self.entries.insert(id.to_owned(), content.to_owned());
            return;
        }
        if self.entries.len() >= Self::MAX_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(id.to_owned(), content.to_owned());
        self.order.push_back(id.to_owned());
    }

    fn get(&self, id: &str) -> Option<&str> {
        self.entries.get(id).map(String::as_str)
    }
}

/// 跟随系统代理变化的独立网络客户端（不设置整体超时，允许长时间流式响应）。
struct ProxyClient {
    cached: std::sync::Mutex<Option<(crate::network::ProxySnapshot, reqwest::Client)>>,
}

impl Default for ProxyClient {
    fn default() -> Self {
        Self {
            cached: std::sync::Mutex::new(None),
        }
    }
}

impl ProxyClient {
    fn current(&self) -> Result<reqwest::Client, AppError> {
        let snapshot = ClientCache::cached_snapshot();
        let mut cached = self
            .cached
            .lock()
            .map_err(|_| AppError::Internal("本机转换代理的网络客户端锁已损坏。".into()))?;
        if cached
            .as_ref()
            .is_some_and(|(cached_snapshot, _)| cached_snapshot == &snapshot)
        {
            return Ok(cached.as_ref().expect("刚刚检查过").1.clone());
        }
        let client = ClientCache::build_standalone(None).map_err(|error| {
            AppError::Internal(format!(
                "无法初始化本机转换代理的网络客户端：{}",
                error.without_url()
            ))
        })?;
        *cached = Some((snapshot, client.clone()));
        Ok(client)
    }
}

/// 生产环境固定使用 PROXY_PORT；测试并行运行时每个用例分配独立端口，避免冲突。
fn bind_port() -> u16 {
    #[cfg(test)]
    {
        test_port()
    }
    #[cfg(not(test))]
    {
        PROXY_PORT
    }
}

#[cfg(test)]
fn test_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(28_000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

async fn start_proxy(
    config: ProxyConfig,
) -> Result<(u16, oneshot::Sender<()>, Arc<ProxyConfigSlot>), AppError> {
    let config_slot = ProxyConfigSlot::new(config);
    let state = ProxyState {
        config: config_slot.clone(),
        client: ProxyClient::default(),
        reasoning_store: Arc::new(std::sync::Mutex::new(ReasoningStore::default())),
    };
    let listener = tokio::net::TcpListener::bind((PROXY_HOST, bind_port()))
        .await
        .map_err(|error| {
            AppError::Internal(format!(
                "无法在本机 {PROXY_HOST}:{PROXY_PORT} 启动转换代理（{error}）。请确认端口 27777 未被其他程序占用后重试。"
            ))
        })?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::Internal(format!("无法读取本机转换代理端口：{error}")))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let app = Router::new()
        .route("/v1/responses", post(handle_responses))
        .route("/v1/models", get(handle_models))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(Arc::new(state));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((address.port(), shutdown_tx, config_slot))
}

/// 服务只提供 Chat Completions API 时，写入 Codex 的 base_url 是本机代理；
/// 直连 Responses API 的服务则写服务商地址本身。
pub(crate) async fn effective_base_url(
    provider: &ProviderProfile,
    registry: &ChatProxyRegistry,
) -> Result<String, AppError> {
    match provider.api_type {
        ProviderApiType::Responses => Ok(provider.base_url.clone()),
        ProviderApiType::Chat => {
            registry.ensure(provider).await?;
            Ok(proxy_base_url())
        }
    }
}

fn apply_upstream_headers(
    mut request: reqwest::RequestBuilder,
    config: &ProxyConfig,
) -> reqwest::RequestBuilder {
    request = request.bearer_auth(&config.api_key);
    // 服务商自定义头在保存时已经过 validate_headers 校验，这里即使遇到
    // 异常值也只跳过错误项，避免本机代理因请求头问题拒绝转发。
    if let Ok(headers) = crate::provider_http::headers_from_pairs(config.headers.clone()) {
        request = request.headers(headers);
    }
    request
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let mut response = Json(json!({
        "error": {"message": message, "type": "upstream_error", "param": null, "code": status.as_u16().to_string()}
    }))
    .into_response();
    *response.status_mut() = status;
    response
}

/// 校验调用方凭证：只有本应用写入 Codex 配置的固定 Key 可以访问转换代理，
/// 防止本机其他进程或网页借用代理消耗用户的真实上游额度。
fn is_authorized(headers: &HeaderMap) -> bool {
    let expected = format!("Bearer {PROXY_FIXED_API_KEY}");
    headers
        .get(reqwest::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str())
}

fn unauthorized_response() -> Response {
    error_response(
        StatusCode::UNAUTHORIZED,
        "缺少或错误的代理访问凭证，请求已拒绝。",
    )
}

async fn handle_models(State(state): State<Arc<ProxyState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers) {
        return unauthorized_response();
    }
    let config = state.config.current.read().await.clone();
    if config.is_disabled() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "本机转换代理尚未配置上游服务，请先在应用中切换一个 Chat Completions 服务。",
        );
    }
    let client = match state.client.current() {
        Ok(client) => client,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    };
    let url = crate::provider_http::endpoint_for(&config.upstream_base, "models");
    // /models 是非流式请求，需要整体超时：转换代理的网络客户端为了
    // 长时间流式响应特意不设总超时，若不在这里兜底，上游建连后一直
    // 不返回会让 Codex 侧的模型列表请求无限悬挂。
    const UPSTREAM_MODELS_TIMEOUT: Duration = Duration::from_secs(30);
    let fetch = async {
        let response = apply_upstream_headers(client.get(&url), &config)
            .send()
            .await
            .map_err(|error| format!("无法连接上游服务获取模型列表：{error}"))?;
        let status = response.status();
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| "上游服务返回的模型列表不是有效 JSON。".to_string())?;
        Ok::<_, String>((status, body))
    };
    match tokio::time::timeout(UPSTREAM_MODELS_TIMEOUT, fetch).await {
        Ok(Ok((status, body))) => (status, Json(body)).into_response(),
        Ok(Err(message)) => error_response(StatusCode::BAD_GATEWAY, &message),
        Err(_) => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "等待上游服务返回模型列表超时。",
        ),
    }
}

async fn handle_responses(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !is_authorized(&headers) {
        return unauthorized_response();
    }
    let body: Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "请求体不是有效的 JSON 对象，无法转换为 Chat Completions 请求。",
            );
        }
    };
    // 先校验代理配置，避免在未配置上游时做无用的请求翻译。
    let config = state.config.current.read().await.clone();
    if config.is_disabled() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "本机转换代理尚未配置上游服务，请先在应用中切换一个 Chat Completions 服务。",
        );
    }
    let stream_requested = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let reasoning_store = state.reasoning_store.clone();
    let chat_body = responses_to_chat_body(&body, &reasoning_store);
    let client = match state.client.current() {
        Ok(client) => client,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    };
    let url = crate::provider_http::endpoint_for(&config.upstream_base, "chat/completions");
    let mut response = match apply_upstream_headers(client.post(&url), &config)
        .json(&chat_body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("无法连接上游服务：{error}"),
            );
        }
    };
    let mut status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        // 部分第三方 API 不支持 response_format（json_schema）结构化输出，
        // 此时降级为“把 JSON Schema 写进系统提示词”的方式重试一次，
        // 让自动审查等依赖结构化输出的功能在第三方模型上也能工作。
        if chat_body.get("response_format").is_some() && looks_like_structured_output_error(&detail)
        {
            let degraded = degrade_structured_output(&chat_body);
            match apply_upstream_headers(client.post(&url), &config)
                .json(&degraded)
                .send()
                .await
            {
                Ok(retry) => {
                    response = retry;
                    status = response.status();
                    if !status.is_success() {
                        let retry_detail = response.text().await.unwrap_or_default();
                        return upstream_error_response(status, &retry_detail);
                    }
                }
                Err(error) => {
                    return error_response(
                        StatusCode::BAD_GATEWAY,
                        &format!("无法连接上游服务：{error}"),
                    );
                }
            }
        } else {
            return upstream_error_response(status, &detail);
        }
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created_at = chrono::Utc::now().timestamp();
    if stream_requested {
        let stream = translate_stream(
            response,
            response_id,
            model,
            created_at,
            reasoning_store.clone(),
        );
        let mut sse_response = Sse::new(stream).into_response();
        sse_response
            .headers_mut()
            .insert("Cache-Control", HeaderValue::from_static("no-cache"));
        sse_response
            .headers_mut()
            .insert("X-Accel-Buffering", HeaderValue::from_static("no"));
        sse_response
    } else {
        let chat_response = match response.json::<Value>().await {
            Ok(value) => value,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "上游服务返回的响应不是有效 JSON。",
                );
            }
        };
        Json(chat_to_responses_body(
            &chat_response,
            &response_id,
            &model,
            created_at,
            &reasoning_store,
        ))
        .into_response()
    }
}

/// 上游返回非成功状态时，尽量把 Chat 风格的错误原样带回给 Codex。
fn upstream_error_response(status: StatusCode, detail: &str) -> Response {
    if let Ok(value) = serde_json::from_str::<Value>(detail)
        && value.get("error").is_some()
    {
        return (status, Json(value)).into_response();
    }
    let message = if detail.trim().is_empty() {
        format!("上游服务返回 HTTP {}。", status.as_u16())
    } else {
        format!(
            "上游服务返回 HTTP {}：{}",
            status.as_u16(),
            truncate(detail, 500)
        )
    };
    error_response(status, &message)
}

fn truncate(value: &str, limit: usize) -> String {
    // 用 char_indices 单趟定位第 limit 个字符的字节边界，
    // 避免为整个字符串分配字符数组（上游错误详情可能很大）。
    let cut = value
        .char_indices()
        .enumerate()
        .find_map(|(count, (index, _))| (count == limit).then_some(index));
    let Some(cut) = cut else {
        return value.to_owned();
    };
    let mut output = String::with_capacity(cut + 1);
    output.push_str(&value[..cut]);
    output.push('…');
    output
}

/// 按 SSE 规范解析上游事件：`data:` 行累积（多行自动以 `\n` 连接），
/// 空行触发一次事件分发；`event:`/`id:`/注释行忽略。
/// 使用单个可复用的 String 累积，避免每个 data 行单独分配。
#[derive(Default)]
struct SseEventParser {
    data: String,
}

impl SseEventParser {
    /// 喂入一行（不含换行符与行尾 `\r`）。返回累积的事件 data（空行触发分发时）。
    fn push_line(&mut self, line: &str) -> Option<String> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.dispatch();
        }
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(data);
        }
        None
    }

    /// 流结束（EOF）时处理残留数据。
    fn finish(&mut self) -> Option<String> {
        self.dispatch()
    }

    fn dispatch(&mut self) -> Option<String> {
        if self.data.is_empty() {
            return None;
        }
        // 取出缓冲区，后续累积直接复用同一块内存。
        Some(std::mem::take(&mut self.data))
    }
}

/// 把字节行转成 `&str`（SSE/JSON 应为 UTF-8，正常路径零分配）；
/// 仅在无效 UTF-8 的罕见路径上做 lossy 转换。
fn utf8_lossy_slice(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(line) => std::borrow::Cow::Borrowed(line),
        Err(_) => std::borrow::Cow::Owned(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// 处理一条完整的事件数据。返回 `true` 表示遇到 [DONE] 或上游错误，应停止读取。
fn dispatch_data(
    translator: &mut StreamTranslator,
    data: &str,
    pending: &mut Vec<PendingEvent>,
    failed: &mut bool,
    done: &mut bool,
) -> bool {
    if data == "[DONE]" {
        *done = true;
        return true;
    }
    if let Ok(chunk) = serde_json::from_str::<Value>(data)
        && translator.push_chunk(&chunk, pending).is_some()
    {
        *failed = true;
        return true;
    }
    false
}

fn translate_stream(
    response: reqwest::Response,
    response_id: String,
    model: String,
    created_at: i64,
    store: Arc<std::sync::Mutex<ReasoningStore>>,
) -> impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<PendingEvent>(64);
    tokio::spawn(async move {
        let mut translator = StreamTranslator::new(&response_id, &model, created_at);
        let mut pending = vec![
            sse_event(
                "response.created",
                &translator.stub("response.created", "in_progress"),
            ),
            sse_event(
                "response.in_progress",
                &translator.stub("response.in_progress", "in_progress"),
            ),
        ];
        // 立即把 created/in_progress 发给客户端，让 Codex 尽早感知连接已建立；
        // 也避免上游（或系统代理）建连较慢时长时间没有任何事件。
        flush_events(&tx, &mut pending).await;

        let mut parser = SseEventParser::default();
        // 预分配接收缓冲，避免逐块扩容；行提取用游标定位，避免每行整体搬运剩余缓冲。
        let mut buffer: Vec<u8> = Vec::with_capacity(8 * 1024);
        let mut stream = response.bytes_stream();
        let mut failed = false;
        let mut done = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => buffer.extend_from_slice(&bytes),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
            let mut start = 0usize;
            let mut scan = 0usize;
            while let Some(relative) = buffer[scan..].iter().position(|&byte| byte == b'\n') {
                let position = scan + relative;
                if let Some(data) =
                    parser.push_line(utf8_lossy_slice(&buffer[start..position]).as_ref())
                    && dispatch_data(&mut translator, &data, &mut pending, &mut failed, &mut done)
                {
                    start = position + 1;
                    break;
                }
                start = position + 1;
                scan = start;
            }
            if !done && !failed && start > 0 {
                // 已消费的行一次性移除（每块只搬运一次剩余部分）；
                // 结束时不搬，直接丢弃。
                buffer.drain(..start);
            }
            flush_events(&tx, &mut pending).await;
            if done || failed {
                break;
            }
        }
        if !done && !failed {
            // 上游没有发送 [DONE] 就断开：先把残留的未换行行喂给解析器，
            // 再分发解析器里累积的数据，保证最后一个事件不丢失。
            if !buffer.is_empty() {
                if let Some(data) = parser.push_line(utf8_lossy_slice(&buffer).as_ref()) {
                    dispatch_data(&mut translator, &data, &mut pending, &mut failed, &mut done);
                }
            }
            if let Some(data) = parser.finish() {
                dispatch_data(&mut translator, &data, &mut pending, &mut failed, &mut done);
            }
        }
        if failed {
            // 上游已在流中报告错误（fail 事件已发出）或连接中断。
            if !translator.has_failed_event {
                translator.fail(&mut pending, "读取上游流式响应失败，连接可能已中断。");
            }
        } else {
            translator.finish(&mut pending, &store);
        }
        flush_events(&tx, &mut pending).await;
        let _ = tx
            .send(PendingEvent {
                event_type: "done",
                data: "[DONE]".into(),
            })
            .await;
        // 发送端随任务结束被丢弃，接收端流随之结束。
    });
    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| {
            (
                Ok::<Event, std::convert::Infallible>(into_axum_event(event)),
                rx,
            )
        })
    })
}

async fn flush_events(
    tx: &tokio::sync::mpsc::Sender<PendingEvent>,
    pending: &mut Vec<PendingEvent>,
) {
    for event in pending.drain(..) {
        // 优先无等待发送；通道满时才让出（背压），减少逐事件 await 的开销。
        match tx.try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
        }
    }
}

/// 翻译过程中暂存的 SSE 事件；发送前再转成 axum 的 `Event`。
/// 单独定义是为了让单元测试可以直接检查事件类型与载荷。
struct PendingEvent {
    event_type: &'static str,
    data: String,
}

fn sse_event(event_type: &'static str, payload: &Value) -> PendingEvent {
    PendingEvent {
        event_type,
        data: serde_json::to_string(payload).unwrap_or_else(|_| "{}".into()),
    }
}

/// 逐 token 的热路径事件：直接构建 JSON 字符串，避免中间 `Value` 分配。
/// `item_id` 是代理生成的十六进制 id（无需转义）；`delta` 用 serde_json 转义。
fn text_delta_event(
    event_type: &'static str,
    item_id: &str,
    output_index: usize,
    sequence: u32,
    delta: &str,
) -> PendingEvent {
    let delta = serde_json::to_string(delta).unwrap_or_else(|_| "\"\"".to_owned());
    PendingEvent {
        event_type,
        data: format!(
            "{{\"type\":\"{event_type}\",\"item_id\":\"{item_id}\",\"output_index\":{output_index},\"content_index\":0,\"sequence_number\":{sequence},\"delta\":{delta}}}"
        ),
    }
}

fn arguments_delta_event(item_id: &str, output_index: usize, delta: &str) -> PendingEvent {
    let delta = serde_json::to_string(delta).unwrap_or_else(|_| "\"\"".to_owned());
    PendingEvent {
        event_type: "response.function_call_arguments.delta",
        data: format!(
            "{{\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"{item_id}\",\"output_index\":{output_index},\"delta\":{delta}}}"
        ),
    }
}

fn into_axum_event(pending: PendingEvent) -> Event {
    Event::default()
        .event(pending.event_type)
        .data(pending.data)
}

// ---------------------------------------------------------------------------
// 请求转换：Responses API → Chat Completions
// ---------------------------------------------------------------------------

fn responses_to_chat_body(body: &Value, store: &std::sync::Mutex<ReasoningStore>) -> Value {
    let mut chat = Map::new();
    if let Some(model) = body.get("model").cloned() {
        chat.insert("model".into(), model);
    }
    let store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    chat.insert(
        "messages".into(),
        Value::Array(input_to_messages(body, &store)),
    );
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if stream {
        chat.insert("stream".into(), Value::Bool(true));
        // 请求用量统计，供 response.completed 事件回填；不支持的平台会忽略该字段。
        chat.insert("stream_options".into(), json!({"include_usage": true}));
    }
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| convert_tools(tools))
        .unwrap_or_default();
    if !tools.is_empty() {
        chat.insert("tools".into(), Value::Array(tools));
        if let Some(parallel) = body.get("parallel_tool_calls").cloned() {
            chat.insert("parallel_tool_calls".into(), parallel);
        }
    }
    if let Some(tool_choice) = body.get("tool_choice") {
        let converted = convert_tool_choice(tool_choice);
        // 默认的 auto 不需要显式发送，部分平台对不认识的参数会直接报错。
        if converted != Value::String("auto".into()) {
            chat.insert("tool_choice".into(), converted);
        }
    }
    if let Some(max_output_tokens) = body.get("max_output_tokens") {
        chat.insert("max_tokens".into(), max_output_tokens.clone());
    }
    for key in ["temperature", "top_p", "store", "metadata", "user"] {
        if let Some(value) = body.get(key).filter(|value| !value.is_null()) {
            chat.insert(key.into(), value.clone());
        }
    }
    if let Some(reasoning) = body.get("reasoning").and_then(Value::as_object)
        && let Some(effort) = reasoning.get("effort").and_then(Value::as_str)
        && !effort.is_empty()
    {
        chat.insert("reasoning_effort".into(), Value::String(effort.into()));
    }
    // Responses 的 text.format.json_schema → Chat 的 response_format。
    if let Some(format) = body
        .get("text")
        .and_then(|text| text.get("format"))
        .filter(|format| format.get("type").and_then(Value::as_str) == Some("json_schema"))
    {
        let mut json_schema = Map::new();
        for key in ["name", "schema", "strict", "description"] {
            if let Some(value) = format.get(key) {
                json_schema.insert(key.into(), value.clone());
            }
        }
        chat.insert(
            "response_format".into(),
            json!({"type": "json_schema", "json_schema": json_schema}),
        );
    }
    Value::Object(chat)
}

/// 判断上游错误是否与结构化输出（response_format / json_schema）相关，
/// 用于决定是否需要降级重试。
fn looks_like_structured_output_error(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    [
        "response_format",
        "json_schema",
        "structured output",
        "structured_output",
        "structured outputs",
        "unavailable now",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
}

/// 把 Chat 请求里的 `response_format`（json_schema）降级为系统提示词指令，
/// 兼容不支持结构化输出的第三方 API：移除 response_format，并把 JSON Schema
/// 追加到 system 消息中要求模型按 schema 输出 JSON。
/// 指令刻意写得强硬并显式列出必填字段，降低模型漏字段/加解释的概率。
fn degrade_structured_output(chat: &Value) -> Value {
    let mut degraded = chat.clone();
    degraded
        .as_object_mut()
        .map(|map| map.remove("response_format"));
    let Some(schema) = chat
        .pointer("/response_format/json_schema/schema")
        .filter(|value| !value.is_null())
        .cloned()
    else {
        return degraded;
    };
    let schema_text = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| schema.to_string());
    let mut instruction = String::from(
        "You MUST respond with ONLY a single valid JSON object. Do not wrap it in markdown \
         code fences, do not add any explanation, preamble, or trailing text before or after the \
         JSON object.\n\
         The JSON object MUST conform exactly to the following JSON Schema:",
    );
    let required = required_fields(&schema);
    if !required.is_empty() {
        instruction.push_str(&format!(
            "\nThe object MUST include every one of these top-level fields: {}.\
             \nDo not omit any field; use the exact field names and value types defined by the schema.",
            required.join(", ")
        ));
    }
    instruction.push_str(&format!("\n{schema_text}"));
    let Some(messages) = degraded.get_mut("messages").and_then(Value::as_array_mut) else {
        return degraded;
    };
    if let Some(system) = messages
        .iter_mut()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    {
        match system.get("content") {
            Some(Value::String(text)) => {
                system["content"] = Value::String(format!("{text}\n\n{instruction}"));
            }
            Some(Value::Array(parts)) => {
                let mut parts = parts.clone();
                parts.push(json!({ "type": "text", "text": instruction }));
                system["content"] = Value::Array(parts);
            }
            _ => {
                system["content"] = Value::String(instruction.clone());
            }
        }
    } else {
        messages.insert(0, json!({ "role": "system", "content": instruction }));
    }
    degraded
}

/// 从 JSON Schema 提取顶层必填字段：优先 `required` 数组，
/// 没有时退回 `properties` 的键名，用于在提示词里显式列出。
fn required_fields(schema: &Value) -> Vec<String> {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !required.is_empty() {
        return required;
    }
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
}

fn input_to_messages(body: &Value, store: &ReasoningStore) -> Vec<Value> {
    // 预分配：大致等于输入条目数 + 可能的 system 消息。
    let capacity = body
        .get("input")
        .and_then(Value::as_array)
        .map_or(1, |items| items.len() + 1);
    let mut messages: Vec<Value> = Vec::with_capacity(capacity);
    if let Some(instructions) = body
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|instructions| !instructions.trim().is_empty())
    {
        messages.push(json!({"role": "system", "content": instructions}));
    }
    match body.get("input") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Some(Value::Array(items)) => {
            let mut pending_tool_calls: Vec<Value> = Vec::new();
            let mut pending_reasoning: Option<String> = None;
            for item in items {
                match item {
                    Value::String(text) => {
                        flush_tool_calls(
                            &mut messages,
                            &mut pending_tool_calls,
                            &mut pending_reasoning,
                        );
                        messages.push(json!({"role": "user", "content": text}));
                    }
                    Value::Object(map) => {
                        let item_type =
                            map.get("type").and_then(Value::as_str).unwrap_or("message");
                        match item_type {
                            "function_call" => {
                                pending_tool_calls.push(json!({
                                    "id": map.get("call_id").or_else(|| map.get("id")).and_then(Value::as_str).unwrap_or(""),
                                    "type": "function",
                                    "function": {
                                        "name": map.get("name").and_then(Value::as_str).unwrap_or(""),
                                        "arguments": map.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
                                    }
                                }));
                                // 同属上一轮 Chat assistant 消息的 thinking 内容：
                                // 由转换代理记住，必须回传给 DeepSeek 等 API。
                                // 先按条目 id 查，条目 id 被重写时回退到 call_id。
                                if pending_reasoning.is_none() {
                                    let by_item = map
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .and_then(|id| store.get(id));
                                    let by_call = map
                                        .get("call_id")
                                        .and_then(Value::as_str)
                                        .and_then(|id| store.get(id));
                                    if let Some(reasoning) = by_item.or(by_call) {
                                        pending_reasoning = Some(reasoning.to_owned());
                                    }
                                }
                            }
                            "function_call_output" => {
                                flush_tool_calls(
                                    &mut messages,
                                    &mut pending_tool_calls,
                                    &mut pending_reasoning,
                                );
                                messages.push(json!({
                                    "role": "tool",
                                    "tool_call_id": map.get("call_id").and_then(Value::as_str).unwrap_or(""),
                                    "content": output_to_text(map.get("output").unwrap_or(&Value::Null)),
                                }));
                            }
                            "message" | "" => {
                                flush_tool_calls(
                                    &mut messages,
                                    &mut pending_tool_calls,
                                    &mut pending_reasoning,
                                );
                                let role =
                                    map.get("role").and_then(Value::as_str).unwrap_or("user");
                                let role = if role == "developer" { "system" } else { role };
                                let mut message = json!({"role": role, "content": message_content_to_chat(map.get("content").unwrap_or(&Value::Null))});
                                if role == "assistant"
                                    && let Some(reasoning) = map
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .and_then(|id| store.get(id))
                                {
                                    message["reasoning_content"] =
                                        Value::String(reasoning.to_owned());
                                }
                                if let Some(tool_calls) = extract_tool_calls_from_content(
                                    map.get("content").unwrap_or(&Value::Null),
                                ) {
                                    message["tool_calls"] = tool_calls;
                                }
                                messages.push(message);
                            }
                            // reasoning、web_search_call、file 等输入项无法映射到
                            // Chat Completions，直接忽略。
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            flush_tool_calls(
                &mut messages,
                &mut pending_tool_calls,
                &mut pending_reasoning,
            );
        }
        _ => {}
    }
    messages
}

fn flush_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Option<String>,
) {
    if pending_tool_calls.is_empty() {
        pending_reasoning.take();
        return;
    }
    let mut message = json!({
        "role": "assistant",
        "content": "",
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    if let Some(reasoning) = pending_reasoning.take() {
        message["reasoning_content"] = Value::String(reasoning);
    }
    messages.push(message);
}

fn output_to_text(output: &Value) -> String {
    match output {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 把 Responses 消息的 content（字符串或内容块数组）转成 Chat 的 content。
fn message_content_to_chat(content: &Value) -> Value {
    match content {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(parts) => {
            let mut converted: Vec<Value> = Vec::new();
            for part in parts {
                let Some(map) = part.as_object() else {
                    continue;
                };
                let part_type = map.get("type").and_then(Value::as_str).unwrap_or("text");
                match part_type {
                    "input_text" | "output_text" | "text" => {
                        if let Some(text) = map.get("text").and_then(Value::as_str) {
                            converted.push(json!({"type": "text", "text": text}));
                        }
                    }
                    "input_image" => {
                        converted.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": map.get("image_url").and_then(Value::as_str).unwrap_or(""),
                                "detail": map.get("detail").cloned().unwrap_or(Value::String("auto".into())),
                            }
                        }));
                    }
                    "refusal" => {
                        if let Some(text) = map.get("refusal").and_then(Value::as_str) {
                            converted.push(json!({"type": "text", "text": text}));
                        }
                    }
                    // function_call 内容块单独转成 tool_calls，不进入 content。
                    _ => {}
                }
            }
            if converted.len() == 1
                && let Some(text) = converted[0].get("text").and_then(Value::as_str)
            {
                Value::String(text.to_owned())
            } else if converted.is_empty() {
                Value::String(String::new())
            } else {
                Value::Array(converted)
            }
        }
        _ => Value::String(String::new()),
    }
}

/// 旧式 Responses 消息把 function_call 放在 content 内容块里，这里转成
/// Chat 的 tool_calls。
fn extract_tool_calls_from_content(content: &Value) -> Option<Value> {
    let parts = content.as_array()?;
    let mut calls = Vec::new();
    for part in parts {
        let map = part.as_object()?;
        if map.get("type").and_then(Value::as_str) != Some("function_call") {
            continue;
        }
        calls.push(json!({
            "id": map.get("call_id").or_else(|| map.get("id")).and_then(Value::as_str).unwrap_or(""),
            "type": "function",
            "function": {
                "name": map.get("name").and_then(Value::as_str).unwrap_or(""),
                "arguments": map.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
            }
        }));
    }
    (!calls.is_empty()).then_some(Value::Array(calls))
}

/// Responses 工具是扁平结构，Chat 需要嵌套在 function 里；同时递归移除
/// `additionalProperties` / `strict`，提高对严格校验平台的兼容性。
fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let map = tool.as_object()?;
            if map.get("type").and_then(Value::as_str) != Some("function") {
                return None;
            }
            let mut function = Map::new();
            if let Some(name) = map.get("name").and_then(Value::as_str) {
                function.insert("name".into(), Value::String(name.into()));
            }
            if let Some(description) = map.get("description").and_then(Value::as_str) {
                function.insert("description".into(), Value::String(description.into()));
            }
            if let Some(parameters) = map.get("parameters").cloned() {
                function.insert("parameters".into(), clean_json_schema(parameters));
            }
            Some(json!({"type": "function", "function": function}))
        })
        .collect()
}

fn clean_json_schema(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut cleaned = Map::new();
            for (key, value) in map {
                if matches!(key.as_str(), "additionalProperties" | "strict") {
                    continue;
                }
                cleaned.insert(key, clean_json_schema(value));
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(clean_json_schema).collect()),
        other => other,
    }
}

fn convert_tool_choice(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(value) => Value::String(value.clone()),
        Value::Object(map) if map.get("type").and_then(Value::as_str) == Some("function") => {
            json!({
                "type": "function",
                "function": {
                    "name": map.get("name").and_then(Value::as_str).unwrap_or(""),
                }
            })
        }
        _ => Value::String("auto".into()),
    }
}

// ---------------------------------------------------------------------------
// 非流式响应转换：Chat Completions → Responses API
// ---------------------------------------------------------------------------

fn chat_to_responses_body(
    chat: &Value,
    response_id: &str,
    model: &str,
    created_at: i64,
    store: &std::sync::Mutex<ReasoningStore>,
) -> Value {
    let choice = chat
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .unwrap_or(&Value::Null);
    let message = choice.get("message").unwrap_or(&Value::Null);
    let content_text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // 记住上一轮 reasoning_content，供下一轮请求回传（DeepSeek 等要求）。
    // 按输出条目 id 与工具 call_id 双重索引，id 被重写时仍能匹配。
    if let Some(reasoning) = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        if let Ok(mut store) = store.lock() {
            store.insert(&format!("msg_{suffix}"), reasoning);
            for (index, tool_call) in tool_calls.iter().enumerate() {
                let fc_id = format!("fc_{suffix}_{index}");
                store.insert(&fc_id, reasoning);
                if let Some(call_id) = tool_call.get("id").and_then(Value::as_str) {
                    store.insert(call_id, reasoning);
                }
            }
        }
    }
    let mut output = vec![json!({
        "id": format!("msg_{suffix}"),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": content_text, "annotations": []}],
    })];
    for (index, tool_call) in tool_calls.iter().enumerate() {
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        output.push(json!({
            "id": format!("fc_{suffix}_{index}"),
            "type": "function_call",
            "status": "completed",
            "call_id": tool_call.get("id").and_then(Value::as_str).unwrap_or(""),
            "name": function.get("name").and_then(Value::as_str).unwrap_or(""),
            "arguments": function.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
        }));
    }
    let usage = chat.get("usage").unwrap_or(&Value::Null);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": "completed",
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": input_tokens,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": output_tokens,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": total_tokens,
        },
        "output_text": content_text,
        "parallel_tool_calls": true,
        "previous_response_id": null,
        "reasoning": {"effort": null, "summary": null},
        "text": {"format": {"type": "text"}},
        "tools": [],
        "tool_choice": "auto",
        "temperature": null,
        "top_p": null,
        "truncation": "disabled",
        "store": false,
        "metadata": {},
        "user": null,
    })
}

// ---------------------------------------------------------------------------
// 流式响应转换：Chat SSE → Responses SSE 事件
// ---------------------------------------------------------------------------

struct StreamTranslator {
    response_id: String,
    model: String,
    created_at: i64,
    msg_item_id: String,
    msg_output_index: Option<usize>,
    full_text: String,
    text_started: bool,
    reasoning_item_id: String,
    reasoning_output_index: Option<usize>,
    reasoning_started: bool,
    /// 本响应累积的完整思考内容（reasoning_content），
    /// 结束后按输出条目 id 存入 store 供下一轮回传。
    reasoning_content: String,
    tool_calls: BTreeMap<usize, ToolCallAcc>,
    next_output_index: usize,
    input_tokens: u64,
    output_tokens: u64,
    sequence: u32,
    has_failed_event: bool,
}

struct ToolCallAcc {
    id: String,
    name: String,
    arguments: String,
    item_id: String,
    output_index: Option<usize>,
    started: bool,
}

impl StreamTranslator {
    fn new(response_id: &str, model: &str, created_at: i64) -> Self {
        Self {
            response_id: response_id.to_owned(),
            model: model.to_owned(),
            created_at,
            msg_item_id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            msg_output_index: None,
            full_text: String::new(),
            text_started: false,
            reasoning_item_id: format!("rs_{}", uuid::Uuid::new_v4().simple()),
            reasoning_output_index: None,
            reasoning_started: false,
            reasoning_content: String::new(),
            tool_calls: BTreeMap::new(),
            next_output_index: 0,
            input_tokens: 0,
            output_tokens: 0,
            sequence: 0,
            has_failed_event: false,
        }
    }

    fn stub(&self, event_type: &str, status: &str) -> Value {
        json!({
            "type": event_type,
            "response": {
                "id": self.response_id,
                "object": "response",
                "created_at": self.created_at,
                "status": status,
                "model": self.model,
                "output": [],
                "usage": null,
            }
        })
    }

    /// 处理一个上游流式分片。返回 `Some(错误信息)` 表示上游在流中报告了错误，
    /// 调用方应停止继续读取并结束本次响应。
    fn push_chunk(&mut self, chunk: &Value, out: &mut Vec<PendingEvent>) -> Option<String> {
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("上游服务在流式响应中报告了错误。");
            self.fail(out, message);
            return Some(message.to_owned());
        }
        if let Some(usage) = chunk.get("usage") {
            self.input_tokens = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(self.input_tokens);
            self.output_tokens = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(self.output_tokens);
        }
        let choices = chunk.get("choices").and_then(Value::as_array)?;
        let choice = choices.first()?;
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        // 思考内容：转发为 reasoning_text.delta。Codex 会忽略该事件（不追踪
        // 推理条目），但事件本身能让连接在长思考期间保持活跃，避免任何一层的
        // 空闲超时把连接掐断。
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            if !self.reasoning_started {
                self.reasoning_started = true;
                self.reasoning_output_index = Some(self.claim_output_index());
            }
            self.reasoning_content.push_str(reasoning);
            let output_index = self.reasoning_output_index.unwrap_or(0);
            self.sequence += 1;
            out.push(text_delta_event(
                "response.reasoning_text.delta",
                &self.reasoning_item_id,
                output_index,
                self.sequence,
                reasoning,
            ));
        }
        if let Some(text) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            self.start_text(out);
            self.full_text.push_str(text);
            self.sequence += 1;
            let output_index = self.msg_output_index.unwrap_or(0);
            out.push(text_delta_event(
                "response.output_text.delta",
                &self.msg_item_id,
                output_index,
                self.sequence,
                text,
            ));
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let function = tool_call.get("function").unwrap_or(&Value::Null);
                let acc = self.tool_calls.entry(index).or_insert_with(|| ToolCallAcc {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                    item_id: format!("fc_{}", uuid::Uuid::new_v4().simple()),
                    output_index: None,
                    started: false,
                });
                if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                    acc.id = id.to_owned();
                }
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    acc.name = name.to_owned();
                }
                let arguments_delta = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                // 首次看到该工具调用（无论参数是否为空）就发出 output_item.added，
                // 否则无参数的工具调用会被整个丢弃，Codex 收不到 function_call。
                if !acc.started {
                    acc.started = true;
                    if acc.output_index.is_none() {
                        let index = self.next_output_index;
                        self.next_output_index += 1;
                        acc.output_index = Some(index);
                    }
                    let output_index = acc.output_index.unwrap_or(0);
                    out.push(sse_event(
                        "response.output_item.added",
                        &json!({
                            "type": "response.output_item.added",
                            "output_index": output_index,
                            "item": {
                                "id": acc.item_id,
                                "type": "function_call",
                                "status": "in_progress",
                                "call_id": acc.id,
                                "name": acc.name,
                                "arguments": "",
                            },
                        }),
                    ));
                }
                if !arguments_delta.is_empty() {
                    acc.arguments.push_str(arguments_delta);
                    let output_index = acc.output_index.unwrap_or(0);
                    out.push(arguments_delta_event(
                        &acc.item_id,
                        output_index,
                        arguments_delta,
                    ));
                }
            }
        }
        None
    }

    fn claim_output_index(&mut self) -> usize {
        let index = self.next_output_index;
        self.next_output_index += 1;
        index
    }

    fn start_text(&mut self, out: &mut Vec<PendingEvent>) {
        if self.text_started {
            return;
        }
        self.text_started = true;
        let output_index = self.claim_output_index();
        self.msg_output_index = Some(output_index);
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {
                    "id": self.msg_item_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                },
            }),
        ));
        out.push(sse_event(
            "response.content_part.added",
            &json!({
                "type": "response.content_part.added",
                "item_id": self.msg_item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []},
            }),
        ));
    }

    fn fail(&mut self, out: &mut Vec<PendingEvent>, message: &str) {
        if self.has_failed_event {
            return;
        }
        self.has_failed_event = true;
        let mut response = self.stub("response.failed", "failed");
        response["response"]["error"] = json!({"message": message, "type": "upstream_error"});
        response["response"]["incomplete_details"] = Value::Null;
        out.push(sse_event("response.failed", &response));
    }

    fn finish(&mut self, out: &mut Vec<PendingEvent>, store: &std::sync::Mutex<ReasoningStore>) {
        // 把本响应的完整思考内容按输出条目 id 与工具 call_id 记住，
        // 供下一轮请求回传（id 被重写时 call_id 仍能匹配）。
        if !self.reasoning_content.trim().is_empty()
            && let Ok(mut store) = store.lock()
        {
            store.insert(&self.msg_item_id, &self.reasoning_content);
            for acc in self.tool_calls.values() {
                store.insert(&acc.item_id, &self.reasoning_content);
                if !acc.id.is_empty() {
                    store.insert(&acc.id, &self.reasoning_content);
                }
            }
        }
        let mut output = Vec::new();
        if self.text_started {
            let output_index = self.msg_output_index.unwrap_or(0);
            let part = json!({"type": "output_text", "text": self.full_text, "annotations": []});
            let item = json!({
                "id": self.msg_item_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [part],
            });
            out.push(sse_event(
                "response.output_text.done",
                &json!({
                    "type": "response.output_text.done",
                    "item_id": self.msg_item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "text": self.full_text,
                    "annotations": [],
                }),
            ));
            out.push(sse_event(
                "response.content_part.done",
                &json!({
                    "type": "response.content_part.done",
                    "item_id": self.msg_item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": part,
                }),
            ));
            out.push(sse_event(
                "response.output_item.done",
                &json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item,
                }),
            ));
            output.push((output_index, item));
        }
        let mut tools = self
            .tool_calls
            .values()
            .filter_map(|acc| acc.output_index.map(|output_index| (output_index, acc)))
            .collect::<Vec<_>>();
        tools.sort_by_key(|(index, _)| *index);
        for (output_index, acc) in tools {
            out.push(sse_event(
                "response.function_call_arguments.done",
                &json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": acc.item_id,
                    "output_index": output_index,
                    "arguments": acc.arguments,
                }),
            ));
            let item = json!({
                "id": acc.item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": acc.id,
                "name": acc.name,
                "arguments": acc.arguments,
            });
            out.push(sse_event(
                "response.output_item.done",
                &json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item,
                }),
            ));
            output.push((output_index, item));
        }
        output.sort_by_key(|(index, _)| *index);
        let output = output.into_iter().map(|(_, item)| item).collect::<Vec<_>>();
        let mut response = json!({
            "type": "response.completed",
            "response": {
                "id": self.response_id,
                "object": "response",
                "created_at": self.created_at,
                "status": "completed",
                "model": self.model,
                "output": output,
                "usage": {
                    "input_tokens": self.input_tokens,
                    "input_tokens_details": {"cached_tokens": 0},
                    "output_tokens": self.output_tokens,
                    "output_tokens_details": {"reasoning_tokens": 0},
                    "total_tokens": self.input_tokens + self.output_tokens,
                },
            }
        });
        response["response"]["error"] = Value::Null;
        response["response"]["incomplete_details"] = Value::Null;
        out.push(sse_event("response.completed", &response));
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(id: &str, base_url: &str) -> ProviderProfile {
        ProviderProfile {
            id: id.into(),
            name: id.into(),
            base_url: base_url.into(),
            headers: Default::default(),
            timeout_secs: 30,
            enabled: true,
            active: false,
            model: String::new(),

            model_context_windows: Default::default(),
            available_models: Default::default(),
            models_dev_meta: Default::default(),
            api_type: ProviderApiType::Chat,
            api_key: Some("secret".into()),
            has_api_key: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn upstream_endpoint_respects_versioned_roots() {
        assert_eq!(
            crate::provider_http::endpoint_for("https://api.deepseek.com", "chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            crate::provider_http::endpoint_for("https://api.deepseek.com/v1", "chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            crate::provider_http::endpoint_for("https://gateway.test/openai/v2", "models"),
            "https://gateway.test/openai/v2/models"
        );
    }

    #[test]
    fn truncate_limits_characters_and_handles_multibyte() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcde", 3), "abc…");
        // 多字节字符按字符截断，不会切坏 UTF-8。
        assert_eq!(truncate("a你b", 2), "a你…");
        assert_eq!(truncate("", 5), "");
        assert_eq!(truncate("abcdef", 0), "…");
    }

    fn empty_reasoning_store() -> std::sync::Mutex<ReasoningStore> {
        std::sync::Mutex::new(ReasoningStore::default())
    }

    #[test]
    fn string_input_and_instructions_become_system_and_user_messages() {
        let chat = responses_to_chat_body(
            &json!({
                "model": "deepseek-chat",
                "instructions": "你是翻译助手",
                "input": "你好",
                "stream": true,
            }),
            &empty_reasoning_store(),
        );
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "你是翻译助手");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "你好");
        assert_eq!(chat["model"], "deepseek-chat");
        assert_eq!(chat["stream"], true);
    }

    #[test]
    fn multi_turn_tool_conversation_is_reordered() {
        let chat = responses_to_chat_body(
            &json!({
                "model": "deepseek-chat",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "查天气"}]},
                    {"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{\"city\":\"北京\"}"},
                    {"type": "function_call_output", "call_id": "call_1", "output": "晴 25℃"},
                    {"type": "message", "role": "user", "content": "然后呢？"}
                ]
            }),
            &empty_reasoning_store(),
        );
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "查天气");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["content"], "晴 25℃");
        assert_eq!(messages[3]["role"], "user");
    }

    #[test]
    fn json_schema_format_is_converted_to_response_format() {
        let chat = responses_to_chat_body(
            &json!({
                "model": "deepseek-v4-pro",
                "input": "hi",
                "text": {
                    "format": {
                        "type": "json_schema",
                        "name": "review",
                        "schema": {"type": "object", "properties": {"ok": {"type": "boolean"}}}
                    }
                }
            }),
            &empty_reasoning_store(),
        );
        let format = &chat["response_format"];
        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["json_schema"]["name"], "review");
        assert_eq!(format["json_schema"]["schema"]["type"], "object");
    }

    #[test]
    fn degrade_structured_output_moves_schema_into_system_prompt() {
        let chat = json!({
            "model": "deepseek-v4-pro",
            "messages": [
                {"role": "system", "content": "You are Codex."},
                {"role": "user", "content": "hi"}
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "review",
                    "schema": {"type": "object", "properties": {"ok": {"type": "boolean"}}}
                }
            }
        });
        let degraded = degrade_structured_output(&chat);
        assert!(degraded.get("response_format").is_none());
        let system = &degraded["messages"][0];
        assert_eq!(system["role"], "system");
        let content = system["content"].as_str().unwrap();
        assert!(content.contains("You are Codex."));
        assert!(content.contains("JSON Schema"));
        // schema 没有 required 时退回 properties 键名，显式列出必填字段。
        assert!(content.contains("top-level fields: ok"));
        assert!(content.contains("ONLY a single valid JSON object"));
        // 用户消息保持原样。
        assert_eq!(degraded["messages"][1]["role"], "user");
        assert_eq!(degraded["messages"][1]["content"], "hi");
    }

    #[test]
    fn degrade_instruction_lists_schema_required_fields() {
        let chat = json!({
            "model": "deepseek-v4-pro",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "review",
                    "schema": {
                        "type": "object",
                        "required": ["outcome", "message"],
                        "properties": {
                            "outcome": {"type": "string", "enum": ["approved", "rejected"]},
                            "message": {"type": "string"}
                        },
                        "additionalProperties": false
                    }
                }
            }
        });
        let degraded = degrade_structured_output(&chat);
        let system = &degraded["messages"][0];
        let content = system["content"].as_str().unwrap();
        // 必填字段（含 outcome）被显式列出，且强调不得遗漏。
        assert!(content.contains("top-level fields: outcome, message"));
        assert!(content.contains("Do not omit any field"));
        assert!(content.contains("\"outcome\""));
        assert!(content.contains("markdown code fences"));
    }

    #[test]
    fn degrade_structured_output_inserts_system_message_when_missing() {
        let chat = json!({
            "model": "deepseek-v4-pro",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "review",
                    "schema": {"type": "object"}
                }
            }
        });
        let degraded = degrade_structured_output(&chat);
        assert!(degraded.get("response_format").is_none());
        assert_eq!(degraded["messages"][0]["role"], "system");
        assert!(
            degraded["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("JSON Schema")
        );
        assert_eq!(degraded["messages"][1]["role"], "user");
    }

    #[test]
    fn structured_output_error_detection_ignores_case_and_variants() {
        assert!(looks_like_structured_output_error(
            "This response_format type is unavailable now"
        ));
        assert!(looks_like_structured_output_error(
            "invalid parameter: response_format"
        ));
        assert!(looks_like_structured_output_error(
            "JSON_Schema is not supported"
        ));
        assert!(looks_like_structured_output_error(
            "structured output not enabled"
        ));
        assert!(!looks_like_structured_output_error("invalid api key"));
        assert!(!looks_like_structured_output_error("rate limit exceeded"));
    }

    #[test]
    fn tools_and_tool_choice_are_converted() {
        let chat = responses_to_chat_body(
            &json!({
                "model": "gpt-4o",
                "input": "hi",
                "tools": [
                    {"type": "function", "name": "f", "description": "desc", "parameters": {"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}, "strict": true}
                ],
                "tool_choice": {"type": "function", "name": "f"},
                "max_output_tokens": 128,
                "reasoning": {"effort": "low"},
                "temperature": 0.3,
            }),
            &empty_reasoning_store(),
        );
        let tools = chat["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "f");
        assert_eq!(tools[0]["function"]["description"], "desc");
        assert!(
            tools[0]["function"]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
        assert!(tools[0]["function"].get("strict").is_none());
        assert_eq!(chat["tool_choice"]["type"], "function");
        assert_eq!(chat["tool_choice"]["function"]["name"], "f");
        assert_eq!(chat["max_tokens"], 128);
        assert_eq!(chat["reasoning_effort"], "low");
        assert_eq!(chat["temperature"], 0.3);
    }

    #[test]
    fn image_input_is_preserved() {
        let chat = responses_to_chat_body(
            &json!({
                "model": "gpt-4o",
                "input": [{"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "这是什么"},
                    {"type": "input_image", "image_url": "data:image/png;base64,xxx", "detail": "high"}
                ]}]
            }),
            &empty_reasoning_store(),
        );
        let content = chat["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,xxx");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn non_streaming_response_is_translated() {
        let translated = chat_to_responses_body(
            &json!({
                "id": "chatcmpl-1",
                "created": 1700000000,
                "model": "deepseek-chat",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "你好！",
                        "tool_calls": [{
                            "id": "call_x",
                            "type": "function",
                            "function": {"name": "get_weather", "arguments": "{\"city\":\"北京\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }),
            "resp_test",
            "deepseek-chat",
            1700000000,
            &empty_reasoning_store(),
        );
        assert_eq!(translated["object"], "response");
        assert_eq!(translated["status"], "completed");
        assert_eq!(translated["id"], "resp_test");
        assert_eq!(translated["created_at"], 1700000000);
        let output = translated["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "你好！");
        assert_eq!(output[1]["type"], "function_call");
        assert_eq!(output[1]["call_id"], "call_x");
        assert_eq!(output[1]["name"], "get_weather");
        assert_eq!(output[1]["arguments"], "{\"city\":\"北京\"}");
        assert_eq!(translated["usage"]["input_tokens"], 10);
        assert_eq!(translated["usage"]["output_tokens"], 5);
        assert_eq!(translated["usage"]["total_tokens"], 15);
        assert_eq!(translated["output_text"], "你好！");
    }

    fn events_to_json(events: Vec<PendingEvent>) -> Vec<Value> {
        events
            .into_iter()
            .filter_map(|event| serde_json::from_str(&event.data).ok())
            .collect()
    }

    #[test]
    fn streaming_translator_emits_spec_events() {
        let mut translator = StreamTranslator::new("resp_x", "deepseek-chat", 1);
        let mut events = Vec::new();
        translator.push_chunk(
            &json!({"choices": [{"delta": {"role": "assistant", "content": "你"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"content": "好"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "f", "arguments": "{\"a\":"}}]}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "1}"}}]}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 7, "completion_tokens": 9}}),
            &mut events,
        );
        translator.finish(&mut events, &empty_reasoning_store());

        let values = events_to_json(events);
        let types = values
            .iter()
            .map(|value| value["type"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_item.added",
                "response.function_call_arguments.delta",
                "response.function_call_arguments.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.function_call_arguments.done",
                "response.output_item.done",
                "response.completed",
            ]
        );
        let message_added = &values[0];
        assert_eq!(message_added["output_index"], 0);
        assert_eq!(message_added["item"]["type"], "message");
        let tool_added = &values[4];
        assert_eq!(tool_added["output_index"], 1);
        assert_eq!(tool_added["item"]["type"], "function_call");
        assert_eq!(tool_added["item"]["call_id"], "call_1");
        let completed = values.last().unwrap();
        assert_eq!(completed["response"]["status"], "completed");
        assert_eq!(completed["response"]["usage"]["input_tokens"], 7);
        assert_eq!(completed["response"]["usage"]["output_tokens"], 9);
        let output = completed["response"]["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "你好");
        assert_eq!(output[1]["type"], "function_call");
        assert_eq!(output[1]["arguments"], "{\"a\":1}");
    }

    #[test]
    fn sse_parser_joins_multi_line_data_and_dispatches_on_blank_line() {
        let mut parser = SseEventParser::default();
        assert!(parser.push_line("data: {\"id\": \"chatcmpl-1\",").is_none());
        assert!(parser.push_line("data: \"choices\": []}").is_none());
        let data = parser.push_line("").unwrap();
        assert_eq!(data, "{\"id\": \"chatcmpl-1\",\n\"choices\": []}");
        // 注释与 event: 行被忽略。
        assert!(parser.push_line(": keep-alive").is_none());
        assert!(parser.push_line("event: message").is_none());
        assert!(parser.push_line("data: [DONE]").is_none());
        assert_eq!(parser.push_line("").unwrap(), "[DONE]");
        // EOF 时残留数据也会被分发。
        assert!(parser.push_line("data: {\"a\":1}").is_none());
        assert_eq!(parser.finish().unwrap(), "{\"a\":1}");
        assert!(parser.finish().is_none());
    }

    #[test]
    fn streaming_translator_emits_failed_event_on_error_chunk() {
        let mut translator = StreamTranslator::new("resp_z", "deepseek-chat", 1);
        let mut events = Vec::new();
        let error = translator
            .push_chunk(
                &json!({"error": {"message": "model overloaded", "type": "server_error"}}),
                &mut events,
            )
            .unwrap();
        assert_eq!(error, "model overloaded");
        let values = events_to_json(events);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["type"], "response.failed");
        assert_eq!(values[0]["response"]["status"], "failed");
        assert_eq!(
            values[0]["response"]["error"]["message"],
            "model overloaded"
        );
    }

    #[test]
    fn streaming_translator_forwards_reasoning_content_deltas() {
        let mut translator = StreamTranslator::new("resp_r", "deepseek-reasoner", 1);
        let mut events = Vec::new();
        translator.push_chunk(
            &json!({"choices": [{"delta": {"reasoning_content": "先分析一下，"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"reasoning_content": "再给出答案。"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"content": "答案是 42"}}]}),
            &mut events,
        );
        translator.finish(&mut events, &empty_reasoning_store());

        let values = events_to_json(events);
        let types = values
            .iter()
            .map(|value| value["type"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "response.reasoning_text.delta",
                "response.reasoning_text.delta",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.completed",
            ]
        );
        // 思考内容占 output_index 0，正文消息占 1。
        assert_eq!(values[0]["output_index"], 0);
        assert_eq!(values[0]["delta"], "先分析一下，");
        assert_eq!(values[1]["delta"], "再给出答案。");
        let message_added = values
            .iter()
            .find(|v| v["type"] == "response.output_item.added")
            .unwrap();
        assert_eq!(message_added["output_index"], 1);
        let completed = values.last().unwrap();
        assert_eq!(
            completed["response"]["output"][0]["content"][0]["text"],
            "答案是 42"
        );
    }

    #[test]
    fn tool_call_without_arguments_is_still_emitted() {
        // 无参数工具调用：只有 id/name，没有 arguments 分片，也必须完整输出。
        let mut translator = StreamTranslator::new("resp_t", "deepseek-chat", 1);
        let mut events = Vec::new();
        translator.push_chunk(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "get_time", "arguments": ""}}]}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
            &mut events,
        );
        translator.finish(&mut events, &empty_reasoning_store());

        let values = events_to_json(events);
        let types = values
            .iter()
            .map(|value| value["type"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(types.contains(&"response.output_item.added"));
        assert!(types.contains(&"response.function_call_arguments.done"));
        assert!(types.contains(&"response.output_item.done"));
        let completed = values
            .iter()
            .find(|value| value["type"] == "response.completed")
            .unwrap();
        let output = completed["response"]["output"].as_array().unwrap();
        let call = output
            .iter()
            .find(|item| item["type"] == "function_call")
            .expect("无参数工具调用也必须出现在输出里");
        assert_eq!(call["name"], "get_time");
        assert_eq!(call["call_id"], "call_1");
    }

    #[test]
    fn stream_translator_persists_reasoning_for_its_output_items() {
        let store = empty_reasoning_store();
        let mut translator = StreamTranslator::new("resp_r", "deepseek-reasoner", 1);
        let mut events = Vec::new();
        translator.push_chunk(
            &json!({"choices": [{"delta": {"reasoning_content": "第一步"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"content": "结果"}}]}),
            &mut events,
        );
        translator.push_chunk(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "f", "arguments": "{}"}}]}}]}),
            &mut events,
        );
        translator.finish(&mut events, &store);

        let guard = store.lock().unwrap();
        // 正文消息与工具调用条目都记住了同一份思考内容，供下一轮回传。
        assert_eq!(guard.get(&translator.msg_item_id), Some("第一步"));
        let tool_item_id = translator.tool_calls[&0].item_id.clone();
        assert_eq!(guard.get(&tool_item_id), Some("第一步"));
        drop(guard);
    }

    #[test]
    fn input_to_messages_attaches_remembered_reasoning_content() {
        let store = empty_reasoning_store();
        {
            let mut guard = store.lock().unwrap();
            guard.insert("msg_1", "第一步思考");
            guard.insert("fc_1", "工具调用前的思考");
        }
        let chat = responses_to_chat_body(
            &json!({
                "model": "deepseek-reasoner",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                    {"type": "message", "id": "msg_1", "role": "assistant", "content": [{"type": "output_text", "text": "好的"}]},
                    {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "call_1", "output": "done"}
                ]
            }),
            &store,
        );
        let messages = chat["messages"].as_array().unwrap();
        // assistant 文本消息带上上一轮记住的 reasoning_content。
        let text_message = messages
            .iter()
            .find(|message| message.get("content").and_then(Value::as_str) == Some("好的"))
            .unwrap();
        assert_eq!(text_message["reasoning_content"], "第一步思考");
        // 工具调用 assistant 消息也带上 reasoning_content（DeepSeek 要求回传）。
        let tool_message = messages
            .iter()
            .find(|message| message.get("tool_calls").is_some())
            .unwrap();
        assert_eq!(tool_message["reasoning_content"], "工具调用前的思考");
    }

    #[test]
    fn reasoning_matches_by_call_id_when_item_id_is_rewritten() {
        // 条目 id 被 Codex 重写（重新生成）时，只要 call_id 一致，仍能回传思考内容。
        let store = empty_reasoning_store();
        {
            let mut guard = store.lock().unwrap();
            guard.insert("call_1", "工具调用前的思考");
        }
        let chat = responses_to_chat_body(
            &json!({
                "model": "deepseek-reasoner",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                    {"type": "function_call", "id": "regenerated_fc", "call_id": "call_1", "name": "f", "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "call_1", "output": "done"}
                ]
            }),
            &store,
        );
        let messages = chat["messages"].as_array().unwrap();
        let tool_message = messages
            .iter()
            .find(|message| message.get("tool_calls").is_some())
            .unwrap();
        assert_eq!(tool_message["reasoning_content"], "工具调用前的思考");
    }

    #[test]
    fn non_streaming_response_persists_reasoning_content() {
        let store = empty_reasoning_store();
        let chat_response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "结果",
                    "reasoning_content": "思考过程",
                    "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]
                }
            }]
        });
        let body = chat_to_responses_body(&chat_response, "resp_1", "deepseek-reasoner", 0, &store);
        let msg_id = body["output"][0]["id"].as_str().unwrap().to_owned();
        let guard = store.lock().unwrap();
        assert_eq!(guard.get(&msg_id), Some("思考过程"));
        // 工具调用条目同样记住（同一 suffix，按调用序号区分）。
        let fc_id = format!("fc_{}_0", msg_id.trim_start_matches("msg_"));
        assert_eq!(guard.get(&fc_id), Some("思考过程"));
    }

    #[test]
    fn sse_parser_accumulates_multiline_data_and_tolerates_cr() {
        let mut parser = SseEventParser::default();
        assert_eq!(parser.push_line("data: {\"type\":\"a\""), None);
        assert_eq!(parser.push_line("data: ,\"more\":1}"), None);
        // event:/注释行忽略。
        assert_eq!(parser.push_line("event: x"), None);
        assert_eq!(parser.push_line(": comment"), None);
        assert_eq!(
            parser.push_line(""),
            Some("{\"type\":\"a\"\n,\"more\":1}".into())
        );
        // 分发后缓冲区已取走，可复用。
        assert!(parser.data.is_empty());
        // 行尾 \r 容错。
        assert_eq!(parser.push_line("data: done\r"), None);
        assert_eq!(parser.push_line(""), Some("done".into()));
    }

    #[test]
    fn sse_parser_finish_dispatchs_remaining_data() {
        let mut parser = SseEventParser::default();
        assert_eq!(parser.push_line("data: tail"), None);
        assert_eq!(parser.finish(), Some("tail".into()));
        assert_eq!(parser.finish(), None);
    }

    #[test]
    fn streaming_translator_handles_tool_only_responses() {
        let mut translator = StreamTranslator::new("resp_y", "deepseek-chat", 1);
        let mut events = Vec::new();
        translator.push_chunk(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "f", "arguments": "{}"}}]}}]}),
            &mut events,
        );
        translator.finish(&mut events, &empty_reasoning_store());

        let values = events_to_json(events);
        let completed = values.last().unwrap();
        let output = completed["response"]["output"].as_array().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], "function_call");
        assert_eq!(output[0]["call_id"], "call_1");
    }

    #[tokio::test]
    async fn proxy_translates_non_streaming_requests_end_to_end() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let request = String::from_utf8_lossy(&request);
            assert!(request.contains("chat/completions"));
            assert!(request.contains("Bearer secret"));
            assert!(request.contains("\"messages\""));
            let body = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"deepseek-chat","choices":[{"index":0,"message":{"role":"assistant","content":"你好"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-chat", "input": "你好", "stream": false}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["object"], "response");
        assert_eq!(body["output"][0]["content"][0]["text"], "你好");
        assert_eq!(body["usage"]["input_tokens"], 3);
        upstream.join().unwrap();

        // 不同的服务复用同一个共享代理：监听端口固定，只切换上游配置。
        let registry = ChatProxyRegistry::default();
        let port2 = registry.ensure(&provider).await.unwrap();
        let mut other = provider.clone();
        other.id = "p2".into();
        other.base_url = "http://127.0.0.1:2/v1".into();
        let port3 = registry.ensure(&other).await.unwrap();
        assert_eq!(port2, port3);
        registry.stop_all().await;
    }

    #[tokio::test]
    async fn proxy_rejects_requests_without_credentials() {
        // 未携带本应用写入 Codex 的固定凭证时直接 401，不转发上游。
        let provider = provider("p", "http://127.0.0.1:2/v1");
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .json(&json!({"model": "deepseek-chat", "input": "hi"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 401);
        // 携带错误凭证同样被拒绝。
        let response = client
            .get(format!("http://127.0.0.1:{port}/v1/models"))
            .bearer_auth("wrong-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn proxy_forwards_upstream_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#"{"error":{"message":"invalid api key","type":"authentication_error","code":"invalid_api_key"}}"#;
            write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-chat", "input": "hi"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 401);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["message"], "invalid api key");
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn proxy_degrades_structured_output_when_upstream_rejects_response_format() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut buf = [0_u8; 8192];
            // 第一次请求：带 response_format，上游拒绝结构化输出。
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut buf);
            let request = String::from_utf8_lossy(&buf);
            assert!(request.contains("response_format"));
            let error_body = r#"{"error":{"message":"This response_format type is unavailable now","type":"invalid_request_error"}}"#;
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                error_body.len(),
                error_body
            )
            .unwrap();
            // 第二次请求：降级后不带 response_format，schema 写进 system 提示词。
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut buf);
            let request = String::from_utf8_lossy(&buf);
            assert!(!request.contains("response_format"));
            assert!(request.contains("JSON Schema"));
            assert!(request.contains("top-level fields: ok"));
            let body = r#"{"id":"chatcmpl-2","object":"chat.completion","created":1700000000,"model":"deepseek-chat","choices":[{"index":0,"message":{"role":"assistant","content":"{\"ok\": true}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({
                "model": "deepseek-chat",
                "input": "hi",
                "text": {
                    "format": {
                        "type": "json_schema",
                        "name": "review",
                        "schema": {"type": "object", "properties": {"ok": {"type": "boolean"}}}
                    }
                }
            }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["output"][0]["content"][0]["text"], "{\"ok\": true}");
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn proxy_translates_streaming_requests_end_to_end() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let chunks = [
                r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"role":"assistant","content":"你"}}]}"#,
                r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"好"}}]}"#,
                r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                r#"data: {"id":"chatcmpl-1","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
                "data: [DONE]",
            ]
            .join("\n\n");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                chunks.len(),
                chunks
            )
            .unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-chat", "input": "你好", "stream": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default(),
            "text/event-stream"
        );
        let text = response.text().await.unwrap();
        let types = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .filter_map(|value| value["type"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.completed",
            ]
        );
        assert!(text.contains("data: [DONE]"));
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn proxy_streams_many_tokens_without_loss_or_reordering() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let total = 2000usize;
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            stream.write_all(header.as_bytes()).unwrap();
            // 大量 token 分小块写入，验证逐块解析不丢 token、顺序正确。
            for i in 0..total {
                let data = format!(
                    "data: {{\"id\":\"c\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{i}\"}}}}]}}\n\n"
                );
                stream.write_all(data.as_bytes()).unwrap();
                if i % 50 == 0 {
                    stream.flush().unwrap();
                }
            }
            stream.write_all(b"data: [DONE]\n\n").unwrap();
            stream.flush().unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-chat", "input": "hi", "stream": true}))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        let deltas = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .filter_map(|value| {
                value
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        // 每个 content delta 都收到且顺序正确（无丢失、无乱序）。
        assert_eq!(deltas.len(), total);
        for (index, delta) in deltas.iter().enumerate() {
            assert_eq!(delta, &index.to_string());
        }
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn reasoning_content_is_passed_back_on_next_turn_end_to_end() {
        // 模拟 DeepSeek 推理模型的两轮工具调用：第一轮流式返回 reasoning + 工具调用，
        // 第二轮必须把 reasoning_content 回传给上游（DeepSeek 的硬性要求）。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut buf = [0_u8; 65536];
            // 第一轮：接收请求，返回流式 reasoning + 工具调用。
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut buf);
            let sse = [
                r#"data: {"id":"c","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"第一步思考"}}]}"#,
                r#"data: {"id":"c","choices":[{"index":0,"delta":{"content":"好的"}}]}"#,
                r#"data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"run_shell","arguments":"{\"cmd\":\"ls\"}"}}]}}]}"#,
                r#"data: {"id":"c","usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#,
                "data: [DONE]",
            ]
            .join("\n\n");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                sse
            )
            .unwrap();
            drop(stream);
            // 第二轮：必须收到带 reasoning_content 的 assistant 工具调用消息。
            buf = [0_u8; 65536];
            let (mut stream, _) = listener.accept().unwrap();
            let read = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap_or("{}")).unwrap();
            let messages = body["messages"].as_array().unwrap();
            let tool_message = messages
                .iter()
                .find(|message| message.get("tool_calls").is_some())
                .expect("第二轮应包含工具调用 assistant 消息");
            assert_eq!(
                tool_message["reasoning_content"], "第一步思考",
                "工具调用消息必须回传 reasoning_content"
            );
            let reply = r#"{"id":"chatcmpl-2","object":"chat.completion","created":1700000000,"model":"deepseek-reasoner","choices":[{"index":0,"message":{"role":"assistant","content":"完成"},"finish_reason":"stop"}],"usage":{"prompt_tokens":6,"completion_tokens":2,"total_tokens":8}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            )
            .unwrap();
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("http://127.0.0.1:{port}/v1/responses");
        // 第一轮：流式请求，拿到输出条目（含代理生成的 id）。
        let response = client
            .post(&base)
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-reasoner", "input": "运行命令", "stream": true}))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        let completed = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find(|value| value["type"] == "response.completed")
            .expect("第一轮应收到 response.completed");
        let output = completed["response"]["output"].as_array().unwrap();
        let message_item = output
            .iter()
            .find(|item| item["type"] == "message")
            .unwrap();
        let function_item = output
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        let msg_id = message_item["id"].as_str().unwrap();
        let fc_id = function_item["id"].as_str().unwrap();
        let call_id = function_item["call_id"].as_str().unwrap();
        // 第二轮：带历史继续请求（id 与第一轮响应一致）。
        let response = client
            .post(&base)
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({
                "model": "deepseek-reasoner",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "运行命令"}]},
                    {"type": "message", "id": msg_id, "role": "assistant", "content": [{"type": "output_text", "text": "好的"}]},
                    {"type": "function_call", "id": fc_id, "call_id": call_id, "name": "run_shell", "arguments": "{\"cmd\":\"ls\"}"},
                    {"type": "function_call_output", "call_id": call_id, "output": "ok"}
                ]
            }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn proxy_completes_when_upstream_closes_without_done() {
        // 模拟“不发送 [DONE] 就断开、最后一行没有换行”的激进上游。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let chunks = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"世界\"}}]}";
            write!(stream, "{}", chunks).unwrap();
            drop(stream);
        });
        let provider = provider("p", &format!("http://{address}"));
        let (port, _shutdown, _slot) = start_proxy(ProxyConfig::from_provider(&provider))
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/responses"))
            .bearer_auth(PROXY_FIXED_API_KEY)
            .json(&json!({"model": "deepseek-chat", "input": "hi", "stream": true}))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        let values = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .collect::<Vec<_>>();
        let types = values
            .iter()
            .map(|value| value["type"].as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        // 没有 [DONE] 也要以 response.completed 正常收尾。
        assert!(types.contains(&"response.completed".to_owned()));
        let completed = values
            .iter()
            .find(|value| value["type"] == "response.completed")
            .unwrap();
        let output = completed["response"]["output"].as_array().unwrap();
        assert_eq!(output[0]["content"][0]["text"], "你好世界");
        upstream.join().unwrap();
    }

    #[tokio::test]
    async fn registry_keeps_port_and_swaps_config_when_parameters_change() {
        let registry = ChatProxyRegistry::default();
        let provider = provider("p", "http://127.0.0.1:1/v1");
        let first = registry.ensure(&provider).await.unwrap();
        assert_eq!(registry.ensure(&provider).await.unwrap(), first);
        // 修改上游地址/Key 后端口必须保持不变，否则 Codex 缓存的地址会失效。
        let mut changed = provider.clone();
        changed.base_url = "http://127.0.0.1:2/v1".into();
        changed.api_key = Some("new-secret".into());
        let second = registry.ensure(&changed).await.unwrap();
        assert_eq!(first, second);
        // 配置确实已更新：代理会带着新 Key 请求新的上游地址。
        let slot = registry
            .single
            .lock()
            .await
            .as_ref()
            .unwrap()
            .config
            .clone();
        let config = slot.current.read().await.clone();
        assert_eq!(config.upstream_base, "http://127.0.0.1:2/v1");
        assert_eq!(config.api_key, "new-secret");
        assert_eq!(PROXY_PORT, 27777);
        registry.stop_all().await;
    }

    #[tokio::test]
    async fn deleting_config_owner_disables_upstream_until_next_switch() {
        let registry = ChatProxyRegistry::default();
        let first = provider("p", "http://127.0.0.1:1/v1");
        let port = registry.ensure(&first).await.unwrap();
        // 删除当前配置所属的服务后，配置被清空但监听仍在（端口不变）。
        registry.stop("p").await;
        let slot = registry
            .single
            .lock()
            .await
            .as_ref()
            .unwrap()
            .config
            .clone();
        assert!(slot.current.read().await.clone().is_disabled());
        // 切换到其他服务后配置恢复。
        let other = provider("p2", "http://127.0.0.1:2/v1");
        assert_eq!(registry.ensure(&other).await.unwrap(), port);
        let slot = registry
            .single
            .lock()
            .await
            .as_ref()
            .unwrap()
            .config
            .clone();
        assert_eq!(
            slot.current.read().await.clone().upstream_base,
            "http://127.0.0.1:2/v1"
        );
        registry.stop_all().await;
    }

    #[tokio::test]
    async fn effective_url_uses_proxy_for_chat_and_direct_for_responses() {
        let registry = ChatProxyRegistry::default();
        let chat = provider("chat", "https://api.deepseek.com/v1");
        let url = effective_base_url(&chat, &registry).await.unwrap();
        assert_eq!(url, format!("http://127.0.0.1:{PROXY_PORT}/v1"));
        assert_eq!(url, proxy_base_url());

        let mut responses = chat.clone();
        responses.api_type = ProviderApiType::Responses;
        let url = effective_base_url(&responses, &registry).await.unwrap();
        assert_eq!(url, "https://api.deepseek.com/v1");
        registry.stop_all().await;
    }
}
