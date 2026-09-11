// test_ui.rs - 内置模型测试页（llama-ui 静态构建 + 本机同源反向代理）
//
// llama-ui（llama.cpp tools/ui 的静态构建）通过相对路径访问 `./props`、
// `./v1/chat/completions` 等接口，要求「页面与 API 同源」。Tauri 壳层无法在
// 自身 origin 上挂路由，因此在 Rust 侧起一个只监听 127.0.0.1 的本地 HTTP 服务：
//   - 静态资源：内嵌的 llama-ui 构建产物（src-tauri/ui/，前端用 iframe 打开）
//   - /props   ：按 llama.cpp 格式合成（vLLM / SGLang 没有该接口，缺失时
//                llama-ui 会把发送按钮永久置灰、页面顶部显示 Server unavailable）
//   - 其余 /v1/*：流式透传到当前运行的模型服务端口（SSE 不缓冲）
//
// 端口固定（17890 起顺延），保证 iframe origin 稳定 —— llama-ui 的会话历史
// 存在 IndexedDB 中，origin（含端口）变化会导致历史“丢失”。

use crate::app_state::AppState;
use crate::common::error::AppError;
use axum::body::{Body, Bytes};
use axum::http::{header, Request, Response, StatusCode};
use futures_util::{Stream, StreamExt};
use include_dir::{include_dir, Dir};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// 内嵌的 llama-ui 静态构建产物
static UI_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/ui");

const UI_PORT_FIRST: u16 = 17890;
const UI_PORT_LAST: u16 = 17899;
/// 代理请求体上限（chat 请求含图片 base64，放宽到 128MB）
const PROXY_BODY_LIMIT: usize = 128 * 1024 * 1024;

/// 代理目标：当前运行中的模型服务
#[derive(Default, Clone)]
struct ProxyTarget {
    port: Option<u16>,
    model_id: Option<String>,
    /// 模型是否支持图片输入（决定 /props 的 modalities.vision，否则 UI 禁止贴图）
    vision: bool,
}

static TARGET: OnceLock<Arc<Mutex<ProxyTarget>>> = OnceLock::new();
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static UI_PORT: OnceLock<u16> = OnceLock::new();
/// 启动互斥：并发调用 ensure_server 时避免各绑一个端口（第二个端口会被丢弃）
static SERVER_START_LOCK: Mutex<()> = Mutex::new(());

fn target() -> Arc<Mutex<ProxyTarget>> {
    TARGET
        .get_or_init(|| Arc::new(Mutex::new(ProxyTarget::default())))
        .clone()
}

/// 本机回环 HTTP 客户端：不走系统代理、不设整体超时（长响应/流式对话）
fn http_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        // 默认无整体超时（长对话/流式场景）；connect 阶段失败由内核快速返回
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("build test_ui http client")
    })
}

/// 启动（幂等）本地测试页服务，返回监听端口
pub fn ensure_server() -> Result<u16, AppError> {
    if let Some(port) = UI_PORT.get() {
        return Ok(*port);
    }
    // 串行化启动过程；持锁期间只有同步的 bind/spawn，不跨 await
    let _guard = SERVER_START_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(port) = UI_PORT.get() {
        return Ok(*port);
    }

    let mut last_err = String::new();
    for port in UI_PORT_FIRST..=UI_PORT_LAST {
        let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        if listener.set_nonblocking(true).is_err() {
            continue;
        }
        tauri::async_runtime::spawn(async move {
            if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
                let _ = axum::serve(listener, build_router()).await;
            }
        });
        let _ = UI_PORT.set(port);
        return Ok(port);
    }
    Err(AppError::Other(format!(
        "测试页本地服务启动失败（端口 {} - {} 均被占用）: {}",
        UI_PORT_FIRST, UI_PORT_LAST, last_err
    )))
}

fn build_router() -> axum::Router {
    axum::Router::new().fallback(handle_any)
}

async fn handle_any(req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().to_string();
    if path == "/props" {
        return props_shim().await;
    }
    if path == "/v1" || path.starts_with("/v1/") {
        return proxy_to_model(req).await;
    }
    // llama.cpp 专有接口（vLLM / SGLang 不存在）：明确 404，避免落入 SPA 回退返回 HTML，
    // 让 llama-ui 走与直连一致的容错分支（工具面板置空、控制台仅一条告警）
    if matches!(
        path.as_str(),
        "/tools" | "/slots" | "/cors-proxy" | "/health" | "/metrics" | "/models"
    ) {
        return json_error(StatusCode::NOT_FOUND, &format!("Not Found: {}", path));
    }
    static_file(&path)
}

// ===== /props：llama.cpp 兼容层 =====

async fn props_shim() -> Response<Body> {
    let (port, model_id, vision) = {
        let t = target();
        let g = t.lock().unwrap_or_else(|e| e.into_inner());
        (g.port, g.model_id.clone(), g.vision)
    };
    let Some(port) = port else {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "没有正在运行的模型");
    };

    let mut model = model_id.unwrap_or_else(|| "model".to_string());
    let mut n_ctx: u64 = 32768;
    if let Ok(resp) = http_client()
        .get(format!("http://127.0.0.1:{}/v1/models", port))
        .send()
        .await
    {
        if let Ok(value) = resp.json::<serde_json::Value>().await {
            if let Some(first) = value.get("data").and_then(|d| d.get(0)) {
                if let Some(id) = first.get("id").and_then(|v| v.as_str()) {
                    model = id.to_string();
                }
                if let Some(len) = first.get("max_model_len").and_then(|v| v.as_u64()) {
                    n_ctx = len;
                }
            }
        }
    }

    let model_json = serde_json::Value::String(model).to_string();
    let props = PROPS_TEMPLATE
        .replace("__N_CTX__", &n_ctx.to_string())
        .replace("__MODEL__", &model_json)
        .replace("__VISION__", if vision { "true" } else { "false" });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(props))
        .unwrap()
}

/// llama.cpp `/props` 兼容模板（占位符：__N_CTX__ / __MODEL__ / __VISION__）
const PROPS_TEMPLATE: &str = r#"{
  "default_generation_settings": {
    "id": 0,
    "id_task": 0,
    "n_ctx": __N_CTX__,
    "speculative": false,
    "is_processing": false,
    "params": {
      "n_predict": -1,
      "seed": 4294967295,
      "temperature": 0.8,
      "dynatemp_range": 0.0,
      "dynatemp_exponent": 1.0,
      "top_k": 40,
      "top_p": 0.95,
      "min_p": 0.05,
      "top_n_sigma": -1.0,
      "xtc_probability": 0.0,
      "xtc_threshold": 0.1,
      "typ_p": 1.0,
      "repeat_last_n": 64,
      "repeat_penalty": 1.0,
      "presence_penalty": 0.0,
      "frequency_penalty": 0.0,
      "dry_multiplier": 0.0,
      "dry_base": 1.75,
      "dry_allowed_length": 2,
      "dry_penalty_last_n": -1,
      "dry_sequence_breakers": ["\n", ":", "\"", "*"],
      "mirostat": 0,
      "mirostat_tau": 5.0,
      "mirostat_eta": 0.1,
      "stop": [],
      "max_tokens": -1,
      "n_keep": 0,
      "n_discard": 0,
      "ignore_eos": false,
      "stream": false,
      "logit_bias": [],
      "n_probs": 0,
      "min_keep": 0,
      "grammar": "",
      "grammar_lazy": false,
      "grammar_triggers": [],
      "preserved_tokens": [],
      "chat_format": "",
      "reasoning_format": "none",
      "reasoning_in_content": false,
      "generation_prompt": "",
      "samplers": ["top_k", "tfs_z", "typical_p", "top_p", "min_p", "temperature"],
      "backend_sampling": false,
      "timings_per_token": false,
      "post_sampling_probs": false,
      "lora": []
    },
    "prompt": "",
    "next_token": {
      "has_next_token": false,
      "has_new_line": false,
      "n_remain": -1,
      "n_decoded": 0,
      "stopping_word": ""
    }
  },
  "total_slots": 1,
  "model_path": __MODEL__,
  "role": "model",
  "modalities": { "vision": __VISION__, "audio": false, "video": false },
  "chat_template": "",
  "bos_token": "<s>",
  "eos_token": "</s>",
  "build_info": "adm-be-test-ui",
  "ui_settings": {},
  "cors_proxy_enabled": false
}"#;

// ===== 反向代理（/v1/*） =====

async fn proxy_to_model(req: Request<Body>) -> Response<Body> {
    let port = {
        let t = target();
        let g = t.lock().unwrap_or_else(|e| e.into_inner());
        g.port
    };
    let Some(port) = port else {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "没有正在运行的模型");
    };

    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let url = format!("http://127.0.0.1:{}{}", port, path_and_query);

    let mut forward_headers = req.headers().clone();
    for key in [
        header::HOST,
        header::CONTENT_LENGTH,
        header::CONNECTION,
        // 不向上游声明压缩能力：本地代理不自动解压，SSE 场景更不该压缩
        header::ACCEPT_ENCODING,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        forward_headers.remove(key);
    }

    let mut body_bytes = match axum::body::to_bytes(req.into_body(), PROXY_BODY_LIMIT).await {
        Ok(b) => b,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("读取请求体失败: {}", e)),
    };

    // llama-ui 依赖 llama.cpp 的 timings 字段显示「prefill / decode 速度」，
    // vLLM / SGLang 不会主动返回 —— 流式对话请求里补一个 OpenAI 标准的
    // `stream_options.include_usage`，由代理侧根据响应流补齐 timings（见 sse_timings_stream）
    let capture_timings = method == axum::http::Method::POST
        && path == "/v1/chat/completions"
        && inject_include_usage(&mut body_bytes);

    let mut builder = http_client().request(method, &url);
    for (key, value) in forward_headers.iter() {
        builder = builder.header(key, value);
    }

    // t0 尽量贴近上游开始处理的时刻（后续用于估算 prefill 时长）
    let t0 = Instant::now();
    let upstream = match builder.body(body_bytes).send().await {
        Ok(r) => r,
        Err(e) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!("模型服务不可达（127.0.0.1:{}）: {}", port, e),
            )
        }
    };

    let status = upstream.status();
    let mut out = Response::builder().status(status);
    for key in [header::CONTENT_TYPE, header::CACHE_CONTROL] {
        if let Some(value) = upstream.headers().get(&key) {
            out = out.header(&key, value.clone());
        }
    }

    let is_sse = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/event-stream"))
        .unwrap_or(false);

    // 流式对话：透传 SSE 的同时统计并在 [DONE] 前追加 timings 记录
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
        if capture_timings && is_sse {
            Box::pin(sse_timings_stream(upstream.bytes_stream(), t0))
        } else {
            Box::pin(upstream.bytes_stream().map(|r| {
                r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            }))
        };

    out.body(Body::from_stream(stream))
        .unwrap_or_else(|_| json_error(StatusCode::INTERNAL_SERVER_ERROR, "构建代理响应失败"))
}

/// 流式请求体注入 `stream_options.include_usage = true`（vLLM / SGLang 均支持），
/// 以便拿到 prompt / completion token 数。请求里已有 stream_options 时不改动。
fn inject_include_usage(body: &mut Bytes) -> bool {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    if value.get("stream").and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    if value.get("stream_options").is_some() {
        return true;
    }
    value["stream_options"] = serde_json::json!({ "include_usage": true });
    match serde_json::to_vec(&value) {
        Ok(bytes) => {
            *body = Bytes::from(bytes);
            true
        }
        Err(_) => false,
    }
}

// ===== SSE timings 合成（llama.cpp 兼容） =====

#[derive(Clone, Copy, Default)]
struct UsageInfo {
    prompt_n: u64,
    completion_n: u64,
    cached_n: u64,
}

struct SseTimingsState {
    buf: Vec<u8>,
    t0: Instant,
    t_first: Option<Instant>,
    t_last: Option<Instant>,
    content_chunks: u64,
    usage: Option<UsageInfo>,
    inner_done: bool,
}

enum RecordAction {
    /// 原样透传
    Forward,
    /// 用重写后的记录替换原记录（如：内容块内联实时 timings）
    Replace(Vec<u8>),
    /// 在记录之前追加一条合成记录（如：[DONE] 前的最终 timings）
    Prepend(Vec<u8>),
}

/// 从缓冲区取出一条完整 SSE 记录（以空行结尾）
fn take_record(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2);
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
    let end = match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (x, y) => x.or(y),
    };
    end.map(|e| buf.drain(..e).collect())
}

fn process_record(record: &[u8], st: &mut SseTimingsState) -> RecordAction {
    let Ok(text) = std::str::from_utf8(record) else {
        return RecordAction::Forward;
    };
    let data_lines: Vec<&str> = text
        .split('\n')
        .filter_map(|line| line.trim_start_matches('\r').strip_prefix("data:"))
        .collect();

    // 结束标记：先把最终 timings 插到 [DONE] 之前
    if data_lines.iter().any(|p| p.trim() == "[DONE]") {
        return match build_timings_record(st) {
            Some(extra) => RecordAction::Prepend(extra),
            None => RecordAction::Forward,
        };
    }

    // 单条 data 的 JSON chunk：解析、统计，并给内容块内联实时 timings
    if data_lines.len() == 1 {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(data_lines[0].trim()) else {
            return RecordAction::Forward;
        };

        if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
            st.usage = Some(UsageInfo {
                prompt_n: usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                completion_n: usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                cached_n: usage
                    .pointer("/prompt_tokens_details/cached_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            });
        }

        let has_content = value
            .pointer("/choices/0/delta")
            .map(|delta| {
                ["content", "reasoning_content"].iter().any(|key| {
                    delta
                        .get(*key)
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        if has_content {
            let now = Instant::now();
            if st.t_first.is_none() {
                st.t_first = Some(now);
            }
            st.t_last = Some(now);
            st.content_chunks += 1;

            // 实时 timings（等价 llama.cpp 的 timings_per_token）：生成中即可显示 decode 速度
            if let Some(timings) = live_timings(st) {
                value["timings"] = timings;
                if let Ok(serialized) = serde_json::to_string(&value) {
                    return RecordAction::Replace(format!("data: {}\n\n", serialized).into_bytes());
                }
            }
        }
    }
    RecordAction::Forward
}

/// 生成中的累计 timings（仅 decode 维度；prompt 数据要等 usage 回来）
fn live_timings(st: &SseTimingsState) -> Option<serde_json::Value> {
    let t_first = st.t_first?;
    let t_last = st.t_last.unwrap_or(t_first);
    let predicted_ms = (t_last.saturating_duration_since(t_first).as_secs_f64() * 1000.0).max(1.0);
    Some(serde_json::json!({
        "predicted_n": st.content_chunks,
        "predicted_ms": predicted_ms.round() as u64,
        "cache_n": st.usage.map(|u| u.cached_n).unwrap_or(0),
    }))
}

/// 生成 llama.cpp 格式的 timings 记录（放在 [DONE] 之前）：
/// prompt_n / prompt_ms ≈ prefill（首 token 时延），predicted_n / predicted_ms ≈ decode
fn build_timings_record(st: &SseTimingsState) -> Option<Vec<u8>> {
    let t_first = st.t_first?;
    let t_last = st.t_last.unwrap_or(t_first);

    let predicted_n = st
        .usage
        .map(|u| u.completion_n)
        .filter(|n| *n > 0)
        .unwrap_or(st.content_chunks);
    if predicted_n == 0 {
        return None;
    }
    let prompt_n = st.usage.map(|u| u.prompt_n).unwrap_or(0);
    let cache_n = st.usage.map(|u| u.cached_n).unwrap_or(0);

    let prompt_ms = t_first.saturating_duration_since(st.t0).as_secs_f64() * 1000.0;
    // 至少 1ms，避免 UI 里 predicted_per_second 除零
    let predicted_ms = (t_last.saturating_duration_since(t_first).as_secs_f64() * 1000.0).max(1.0);

    let record = serde_json::json!({
        "choices": [],
        "timings": {
            "prompt_n": prompt_n,
            "prompt_ms": prompt_ms.round() as u64,
            "predicted_n": predicted_n,
            "predicted_ms": predicted_ms.round() as u64,
            "cache_n": cache_n,
        }
    });
    Some(format!("data: {}\n\n", record).into_bytes())
}

fn sse_timings_stream<S>(inner: S, t0: Instant) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let state = SseTimingsState {
        buf: Vec::new(),
        t0,
        t_first: None,
        t_last: None,
        content_chunks: 0,
        usage: None,
        inner_done: false,
    };
    futures_util::stream::unfold((Box::pin(inner), state), |(mut inner, mut st)| async move {
        loop {
            if let Some(record) = take_record(&mut st.buf) {
                return match process_record(&record, &mut st) {
                    RecordAction::Forward => Some((Ok(Bytes::from(record)), (inner, st))),
                    RecordAction::Replace(new_record) => {
                        Some((Ok(Bytes::from(new_record)), (inner, st)))
                    }
                    RecordAction::Prepend(extra) => {
                        // 合成记录必须在原记录之前下发（如 timings 先于 [DONE]）
                        let mut out = extra;
                        out.extend_from_slice(&record);
                        Some((Ok(Bytes::from(out)), (inner, st)))
                    }
                };
            }
            if st.inner_done {
                if !st.buf.is_empty() {
                    let tail: Vec<u8> = st.buf.drain(..).collect();
                    let out = match process_record(&tail, &mut st) {
                        RecordAction::Forward => tail,
                        RecordAction::Replace(new_record) => new_record,
                        RecordAction::Prepend(extra) => {
                            let mut out = extra;
                            out.extend_from_slice(&tail);
                            out
                        }
                    };
                    return Some((Ok(Bytes::from(out)), (inner, st)));
                }
                return None;
            }
            match inner.next().await {
                Some(Ok(chunk)) => st.buf.extend_from_slice(&chunk),
                Some(Err(e)) => {
                    st.inner_done = true;
                    return Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("上游流中断: {}", e),
                        )),
                        (inner, st),
                    ));
                }
                None => st.inner_done = true,
            }
        }
    })
}

// ===== 静态资源 =====

fn static_file(path: &str) -> Response<Body> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };

    if let Some(file) = UI_DIR.get_file(rel) {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime_for(rel))
            .body(Body::from(file.contents()))
            .unwrap();
    }

    // SPA 回退：无扩展名的路径（/chat/<id>、/settings 等）回退 index.html
    let has_extension = rel
        .rsplit('/')
        .next()
        .map(|name| name.contains('.'))
        .unwrap_or(false);
    if !has_extension {
        if let Some(file) = UI_DIR.get_file("index.html") {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(file.contents()))
                .unwrap();
        }
    }

    json_error(StatusCode::NOT_FOUND, &format!("Not Found: {}", path))
}

fn mime_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(
            serde_json::json!({ "error": { "message": message, "type": "test_ui_error" } })
                .to_string(),
        ))
        .unwrap()
}

// ===== Tauri 命令 =====

/// 启动/指向内置测试页：返回 iframe 打开的地址（同源页面 + 代理到模型端口）
#[tauri::command]
pub async fn start_test_ui(
    state: tauri::State<'_, AppState>,
    port: Option<u16>,
    model_id: Option<String>,
    vision: Option<bool>,
) -> Result<String, AppError> {
    let server_port = ensure_server()?;

    // 未显式传参时取后端记录的在运行模型
    let running_port = state
        .running_port
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let running_id = state
        .running_model_id
        .lock()
        .map_err(|e| e.to_string())?
        .clone();

    {
        let t = target();
        let mut g = t.lock().unwrap_or_else(|e| e.into_inner());
        g.port = port.or(running_port);
        g.model_id = model_id.or(running_id);
        g.vision = vision.unwrap_or(false);
    }

    Ok(format!("http://127.0.0.1:{}/", server_port))
}

/// 释放测试页与模型的绑定（模型停止时调用；本地服务本身常驻，端口保持稳定）
#[tauri::command]
pub async fn stop_test_ui() -> Result<(), AppError> {
    let t = target();
    let mut g = t.lock().unwrap_or_else(|e| e.into_inner());
    g.port = None;
    g.model_id = None;
    Ok(())
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};

    /// 全局 TARGET 为进程级状态，测试间串行避免互相干扰
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    async fn spawn_server(router: axum::Router) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind random port");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        port
    }

    fn set_target(port: u16) {
        let t = target();
        let mut g = t.lock().unwrap();
        g.port = Some(port);
        g.model_id = Some("mock-7b".to_string());
        g.vision = true;
    }

    /// 模拟 vLLM：/v1/models + 流式 /v1/chat/completions（含 usage 块），其余 404
    /// 返回 (端口, 收到的 chat 请求体)
    async fn spawn_mock_upstream() -> (u16, Arc<Mutex<Option<String>>>) {
        let seen_body: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let seen_body_clone = seen_body.clone();
        let router = axum::Router::new()
            .route(
                "/v1/models",
                get(|| async {
                    axum::Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "id": "mock-7b", "object": "model", "max_model_len": 4096 }]
                    }))
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move |body: String| {
                    let seen = seen_body_clone.clone();
                    async move {
                        *seen.lock().unwrap() = Some(body);
                        let stream = futures_util::stream::iter(vec![
                        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                            b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
                        )),
                        Ok(axum::body::Bytes::from_static(
                            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
                        )),
                        Ok(axum::body::Bytes::from_static(
                            b"data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
                        )),
                        Ok(axum::body::Bytes::from_static(
                            b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                        )),
                        Ok(axum::body::Bytes::from_static(
                            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":2,\"total_tokens\":13,\"prompt_tokens_details\":{\"cached_tokens\":3}}}\n\n",
                        )),
                        Ok(axum::body::Bytes::from_static(b"data: [DONE]\n\n")),
                    ]);
                    Response::builder()
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from_stream(stream))
                        .unwrap()
                    }
                }),
            );
        (spawn_server(router).await, seen_body)
    }

    #[tokio::test]
    async fn static_props_and_proxy_work() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (up_port, seen_body) = spawn_mock_upstream().await;
        set_target(up_port);
        let ui_port = spawn_server(build_router()).await;
        let base = format!("http://127.0.0.1:{}", ui_port);
        let client = reqwest::Client::new();

        // 1) 静态资源：首页 + 内嵌 bundle
        let resp = client.get(format!("{}/", base)).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let html = resp.text().await.unwrap();
        assert!(html.contains("_app/immutable/bundle"), "index.html 应包含 bundle 引用");

        // 2) SPA 回退（无扩展名路由）
        let resp = client.get(format!("{}/chat/abc", base)).send().await.unwrap();
        assert_eq!(resp.status(), 200);

        // 3) 缺失的带扩展名资源 -> 404
        let resp = client.get(format!("{}/missing.js", base)).send().await.unwrap();
        assert_eq!(resp.status(), 404);

        // 3b) llama.cpp 专有接口 -> 404 JSON（不能落入 SPA 回退返回 HTML）
        let resp = client.get(format!("{}/tools", base)).send().await.unwrap();
        assert_eq!(resp.status(), 404);
        assert!(resp.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("application/json"));

        // 4) /props 兼容层：n_ctx 来自 max_model_len，modalities.vision 来自入参
        let resp = client
            .get(format!("{}/props?autoload=false", base))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let props: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(props["default_generation_settings"]["n_ctx"], 4096);
        assert_eq!(props["modalities"]["vision"], true);
        assert_eq!(props["model_path"], "mock-7b");
        assert_eq!(props["role"], "model");

        // 5) /v1/* 透传（含 SSE 流式响应 + timings 合成）
        let resp = client.get(format!("{}/v1/models", base)).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let resp = client
            .post(format!("{}/v1/chat/completions", base))
            .json(&serde_json::json!({ "stream": true, "messages": [{ "role": "user", "content": "hi" }] }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 5a) 请求体被注入 stream_options.include_usage
        let echoed = seen_body.lock().unwrap().clone().unwrap_or_default();
        assert!(
            echoed.contains("\"include_usage\":true"),
            "应注入 stream_options.include_usage，实际: {}",
            echoed
        );

        let text = resp.text().await.unwrap();
        assert!(text.contains("data: "), "SSE 数据应透传");
        assert!(text.contains("\"content\":\"hi\""), "内容块应透传并内联实时 timings");

        // 5b) 内容块内联了实时 timings（等价 timings_per_token）
        let live_line = text
            .lines()
            .find(|l| l.contains("\"content\":\"hi\""))
            .unwrap();
        let live: serde_json::Value =
            serde_json::from_str(live_line.trim_start_matches("data: ")).unwrap();
        assert!(live["timings"]["predicted_n"].as_u64().unwrap_or(0) >= 1);
        assert!(live["timings"]["predicted_ms"].as_u64().unwrap_or(0) >= 1);

        // 5c) [DONE] 之前追加最终 timings 记录，数值来自 usage
        let done_idx = text.find("[DONE]").expect("应包含 [DONE]");
        let timings_idx = text.find("\"prompt_n\"").expect("应合成最终 timings");
        assert!(timings_idx < done_idx, "最终 timings 必须出现在 [DONE] 之前");
        let final_line = text
            .lines()
            .find(|l| l.contains("\"prompt_n\""))
            .unwrap()
            .trim_start_matches("data: ");
        let chunk: serde_json::Value = serde_json::from_str(final_line).unwrap();
        assert_eq!(chunk["timings"]["prompt_n"], 11);
        assert_eq!(chunk["timings"]["predicted_n"], 2);
        assert_eq!(chunk["timings"]["cache_n"], 3);
        assert!(chunk["timings"]["prompt_ms"].as_u64().is_some());
        assert!(chunk["timings"]["predicted_ms"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test]
    async fn proxy_without_model_returns_503() {
        let _guard = TEST_LOCK.lock().unwrap();
        let t = target();
        t.lock().unwrap().port = None;
        let ui_port = spawn_server(build_router()).await;
        let client = reqwest::Client::new();

        let resp = client
            .get(format!("http://127.0.0.1:{}/props", ui_port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
        let resp = client
            .post(format!("http://127.0.0.1:{}/v1/chat/completions", ui_port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
    }
}
