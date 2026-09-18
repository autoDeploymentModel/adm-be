//! 引擎就绪后的自动预热（boot warmup）。
//!
//! vLLM / SGLang 首次执行某类形状的请求时会触发 TileLang/Triton/采样器等内核的 JIT 编译，
//! 导致「启动后第一轮对话」明显卡顿（社区 kit MiaAI-Lab GLM-5.3-Flash EXL3 的 post-ready
//! shape warmup 即为解决同类问题）。这里在引擎就绪后自动发一小组请求，把常见形状的编译
//! 前移到启动阶段：
//! - `/v1/completions` 阶梯：prompt≈1 / 256 / 3584 / 7168 / 14336 token（max_tokens=1），
//!   覆盖小请求与分块 prefill（半块 / 满块 / 两块）形状；
//! - `/v1/chat/completions`：单发 + 4 并发小批，覆盖对话模板与采样器批量形状。
//!
//! 全部尽力而为：任一请求失败只记日志（tag `WARMUP`），不影响启动、不阻塞用户
//! （预热期间用户请求与预热请求共享引擎队列）。模型停止/切换后预热任务自动退出。

use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::common::utils::logger::model_log;

/// `/v1/completions` 预热阶梯（token 近似值；prompt 用重复 `hello` 生成，约 1 token/词）
const COMPLETION_RUNGS: [usize; 5] = [1, 256, 3584, 7168, 14336];
/// 对话预热并发数（对齐目录常用 max_num_seqs=4 的批量形状；并发上限更低的模型自动排队）
const CHAT_BURST: usize = 4;
/// 单请求超时（对齐社区 kit `GLM53_WARMUP_REQ_TIMEOUT` 默认 240s）
const REQ_TIMEOUT_SECS: u64 = 240;
/// `/v1/models` 就绪等待：就绪 banner 可能早于 API 监听（权重加载中），轮询等待；
/// 每次探测 5s 超时、间隔 10s，最长约 15 分钟
const MODEL_WAIT_TRIES: usize = 90;
const MODEL_WAIT_INTERVAL_SECS: u64 = 10;

/// 就绪后异步触发预热：立即返回，后台任务的任何失败都只记日志。
pub fn spawn(app: AppHandle, model_id: String, port: u16, engine: &'static str) {
    let _ = tauri::async_runtime::spawn(async move {
        if let Err(e) = run(&app, &model_id, port, engine).await {
            model_log(
                &app,
                &model_id,
                "WARN",
                "WARMUP",
                &format!("[warmup] 预热未完成：{}", e),
                "stdout",
            );
        }
    });
}

/// 模型是否仍为当前运行实例（停止/切换/重启后预热静默退出，避免对旧/新实例重复预热）。
fn still_current(app: &AppHandle, model_id: &str, generation: u64) -> bool {
    let state = app.state::<AppState>();
    let same_model = state
        .running_model_id
        .lock()
        .map(|m| m.as_deref() == Some(model_id))
        .unwrap_or(false);
    same_model && state.get_model_generation() == generation
}

async fn run(app: &AppHandle, model_id: &str, port: u16, engine: &str) -> Result<(), String> {
    let generation = app.state::<AppState>().get_model_generation();
    let base = format!("http://127.0.0.1:{}", port);
    // 本地回环请求：独立客户端、不走用户代理；单请求超时与社区 kit 对齐
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQ_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;

    // 等待 API 可用并取 served model 名（vLLM / SGLang 会校验请求体里的 model 字段）
    let mut model: Option<String> = None;
    for _ in 0..MODEL_WAIT_TRIES {
        if !still_current(app, model_id, generation) {
            return Ok(());
        }
        if let Ok(resp) = client
            .get(format!("{}/v1/models", base))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<serde_json::Value>().await {
                    model = v
                        .pointer("/data/0/id")
                        .and_then(|x| x.as_str())
                        .map(str::to_string);
                }
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(MODEL_WAIT_INTERVAL_SECS)).await;
    }
    let Some(model) = model else {
        return Err("等待 /v1/models 超时（引擎未就绪或已停止）".to_string());
    };

    model_log(
        app,
        model_id,
        "INFO",
        "WARMUP",
        &format!(
            "[warmup] {} 已就绪，开始预热（首轮 JIT 编译前移；失败不影响使用）",
            engine
        ),
        "stdout",
    );
    let started = Instant::now();
    let mut ok = 0usize;
    let mut total = 0usize;

    // 1) completions 阶梯：小请求 + 分块 prefill 形状
    for s in COMPLETION_RUNGS {
        if !still_current(app, model_id, generation) {
            return Ok(());
        }
        total += 1;
        let payload = json!({
            "model": model,
            "prompt": "hello ".repeat(s),
            "max_tokens": 1,
            "temperature": 0,
        });
        let t0 = Instant::now();
        match post_json(&client, format!("{}/v1/completions", base), &payload).await {
            Ok(()) => {
                ok += 1;
                model_log(
                    app,
                    model_id,
                    "INFO",
                    "WARMUP",
                    &format!(
                        "[warmup] completions s={} 完成（{:.1}s）",
                        s,
                        t0.elapsed().as_secs_f32()
                    ),
                    "stdout",
                );
            }
            Err(e) => {
                model_log(
                    app,
                    model_id,
                    "WARN",
                    "WARMUP",
                    &format!("[warmup] completions s={} 跳过：{}", s, e),
                    "stdout",
                );
            }
        }
    }

    // 2) 对话：单发 + 并发小批（覆盖 chat 模板 / 采样器批量形状）
    let chat_body = |content: &str| {
        json!({
            "model": model,
            "messages": [{ "role": "user", "content": content }],
            "max_tokens": 8,
            "temperature": 0,
        })
    };
    total += 1;
    match post_json(
        &client,
        format!("{}/v1/chat/completions", base),
        &chat_body("hi"),
    )
    .await
    {
        Ok(()) => {
            ok += 1;
            model_log(
                app,
                model_id,
                "INFO",
                "WARMUP",
                "[warmup] chat 单发 完成",
                "stdout",
            );
        }
        Err(e) => {
            model_log(
                app,
                model_id,
                "WARN",
                "WARMUP",
                &format!("[warmup] chat 单发 跳过：{}", e),
                "stdout",
            );
        }
    }
    let mut set = tokio::task::JoinSet::new();
    for i in 0..CHAT_BURST {
        total += 1;
        let c = client.clone();
        let url = format!("{}/v1/chat/completions", base);
        let payload = chat_body(&format!("warmup {}", i + 1));
        set.spawn(async move { post_json(&c, url, &payload).await });
    }
    let mut burst_ok = 0usize;
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => burst_ok += 1,
            Ok(Err(e)) => model_log(
                app,
                model_id,
                "WARN",
                "WARMUP",
                &format!("[warmup] chat 并发批 跳过：{}", e),
                "stdout",
            ),
            Err(e) => model_log(
                app,
                model_id,
                "WARN",
                "WARMUP",
                &format!("[warmup] chat 并发批 任务异常：{}", e),
                "stdout",
            ),
        }
    }
    ok += burst_ok;
    model_log(
        app,
        model_id,
        "INFO",
        "WARMUP",
        &format!("[warmup] chat 并发批 {}/{} 完成", burst_ok, CHAT_BURST),
        "stdout",
    );

    let level = if ok == total { "INFO" } else { "WARN" };
    model_log(
        app,
        model_id,
        level,
        "WARMUP",
        &format!(
            "[warmup] 预热完成 {}/{} 请求（用时 {:.1}s）{}",
            ok,
            total,
            started.elapsed().as_secs_f32(),
            if ok == total {
                ""
            } else {
                "；失败项已跳过，不影响启动"
            }
        ),
        "stdout",
    );
    Ok(())
}

/// POST JSON 并消费响应体；非 2xx 返回可读错误（截断响应文本）。
async fn post_json(
    client: &reqwest::Client,
    url: String,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let resp = client
        .post(url)
        .json(payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "HTTP {} {}",
            status.as_u16(),
            body.chars().take(160).collect::<String>()
        ))
    }
}
