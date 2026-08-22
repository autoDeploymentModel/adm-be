// benchmark.rs - 模型性能测试

use crate::common::error::AppError;
use crate::app_state::AppState;
use crate::bail;
use tauri::Manager;
use tauri::Emitter;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkParams {
    pub input_len: Option<u64>,
    pub output_len: Option<u64>,
    pub num_prompts: Option<u64>,
}

/// 在 SGLang 容器内执行 bench_serving 性能测试
#[tauri::command]
pub async fn start_benchmark(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    params: BenchmarkParams,
) -> Result<(), AppError> {
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    let port = state.running_port.lock().map_err(|e| e.to_string())?.unwrap_or(5678);

    let container_name = container.ok_or("没有正在运行的模型容器")?;
    let model_id = state.running_model_id.lock().map_err(|e| e.to_string())?.clone().unwrap_or_else(|| "default".to_string());

    {
        let mut running = state.benchmark_running.lock().map_err(|e| e.to_string())?;
        if *running {
            bail!("测试正在进行中，请等待完成");
        }
        *running = true;
    }

    let input_len = params.input_len.unwrap_or(1024);
    let output_len = params.output_len.unwrap_or(256);
    let num_prompts = params.num_prompts.unwrap_or(5);

    let base_url = format!("http://127.0.0.1:{}", port);

    // 容器通常无外网，random 数据集默认会下载 ShareGPT 语料；先写入一份本地 JSON 供 --dataset-path 使用
    let _ = crate::common::utils::platform::docker_cmd()
        .args([
            "exec", &container_name,
            "sh", "-c",
            r#"python3 -c 'import json;json.dump([{"conversations":[{"value":"hello world, this is a benchmark prompt from admapp"},{"value":"hi there, this is the assistant reply"}]} for _ in range(20)], open("/tmp/bench_sharegpt.json","w",encoding="utf-8"))'"#,
        ])
        .output();

    let mut cmd = crate::common::utils::platform::docker_cmd();
    cmd.args([
        "exec", "-e", "HF_HUB_OFFLINE=1", "-e", "PYTHONWARNINGS=ignore::FutureWarning", &container_name,
        "python3", "-m", "sglang.benchmark.serving",
        "--backend", "sglang",
        "--base-url", &base_url,
        "--model", "default",
        "--tokenizer", &format!("/models/{}", model_id),
        "--dataset-name", "random",
        "--dataset-path", "/tmp/bench_sharegpt.json",
        "--random-input-len", &input_len.to_string(),
        "--random-output-len", &output_len.to_string(),
        "--num-prompts", &num_prompts.to_string(),
    ]);

    let app_clone = app.clone();

    std::thread::spawn(move || {
        let mut child = match cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = app_clone.emit("benchmark-log", serde_json::json!({"line": format!("[ERROR] 启动测试失败: {}", e)}));
                let _ = app_clone.emit("benchmark-complete", serde_json::json!({"success": false, "output": format!("启动失败: {}", e)}));
                let state = app_clone.state::<AppState>();
                if let Ok(mut r) = state.benchmark_running.lock() { *r = false; }
                return;
            }
        };

        use std::io::{BufRead, BufReader};

        // stdout 线程
        let app_stdout = app_clone.clone();
        let stdout_handle = if let Some(stdout) = child.stdout.take() {
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("INFO", "BENCH", &line);
                    let _ = app_stdout.emit("benchmark-log", serde_json::json!({"line": line}));
                }
            }))
        } else { None };

        // stderr 线程
        let app_stderr = app_clone.clone();
        let stderr_handle = if let Some(stderr) = child.stderr.take() {
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("WARN", "BENCH", &line);
                    let _ = app_stderr.emit("benchmark-log", serde_json::json!({"line": line}));
                }
            }))
        } else { None };

        if let Some(h) = stdout_handle { let _ = h.join(); }
        if let Some(h) = stderr_handle { let _ = h.join(); }

        let status = child.wait();
        let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

        let _ = app_clone.emit("benchmark-complete", serde_json::json!({"success": success}));

        {
            let state = app_clone.state::<AppState>();
            let guard = state.benchmark_running.lock();
            if let Ok(g) = guard {
                let mut g = g;
                *g = false;
            }
        }
    });

    Ok(())
}

/// 查询测试是否正在进行
#[tauri::command]
pub async fn get_benchmark_status(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    let running = state.benchmark_running.lock().map_err(|e| e.to_string())?;
    Ok(*running)
}
