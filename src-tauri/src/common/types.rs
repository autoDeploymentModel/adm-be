use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
pub struct SystemInfo {
    pub total_ram: u64,
    pub used_ram: u64,
    pub total_vram: u64,
    pub used_vram: u64,
    pub has_gpu: bool,
    pub cpu_usage: f32,
    pub cpu_physical_cores: usize,
    pub cpu_logical_cores: usize,
}

#[derive(Serialize, Clone)]
pub struct ModelStatus {
    pub running: bool,
    pub model_id: Option<String>,
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LaunchParams {
    pub ctx_size: Option<i32>,
    /// 监听端口（仅作为历史配置兼容入口；UI 已不再允许修改，后端始终使用 5678）
    pub port: Option<u16>,
    /// 监听地址，如 "127.0.0.1" / "0.0.0.0"
    pub host: Option<String>,
}


#[derive(Serialize, Deserialize, Clone)]
pub struct RemoteModel {
    pub model_id: String,
    /// 旧格式：单文件下载地址（GGUF），新格式模型为空
    #[serde(default)]
    pub model_url: String,
    /// 新格式：HF 仓库多文件下载清单（safetensors 目录模型）
    #[serde(default)]
    pub model_download_files: Vec<String>,
    /// 适配机型列表（如 "dgx-spark-128G"）；空数组 = 所有机型可用
    #[serde(default)]
    pub model_support_devices: Vec<String>,
    /// 模型指定的 SGLang 镜像版本（如 "v0.5.17"）；非空时启动拼成 lmsysorg/sglang:<版本>
    #[serde(default, rename = "sglang-version", alias = "sglang_version")]
    pub sglang_version: String,
    #[serde(default)]
    pub model_size: String,
    #[serde(default)]
    pub model_type: String,
    #[serde(default)]
    pub model_description: String,
    #[serde(default)]
    pub need_ram: String,
    #[serde(default)]
    pub support_tools: bool,
    #[serde(default)]
    pub support_reasoning: bool,
    #[serde(default)]
    pub support_images: bool,
    #[serde(default)]
    pub model_mmproj: Option<String>,
    #[serde(default)]
    pub model_diffusion: Option<String>,
    #[serde(default)]
    pub model_vae: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    pub launch_params: LaunchParams,
    /// SGLang 详细参数配置（Docker 部署用）
    #[serde(default)]
    pub sglang_args: SglangArgs,
    /// 调试模式：开启后在软件根目录记录 API/SSE 交互日志（每次重启自动清空）
    #[serde(default)]
    pub debug_logging: bool,
    /// 界面语言（"zh" 中文 / "en" English，空或未知回退中文）
    #[serde(default)]
    pub language: String,
}

/// SGLang launch_server 详细参数（设置页可手动修改，启动时拼成 `--key value`）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SglangArgs {
    /// Docker 镜像（固定 tag，如 lmsysorg/sglang:v0.5.17）
    #[serde(default = "default_sglang_image")]
    pub image: String,
    /// docker --shm-size
    #[serde(default = "default_sglang_shm")]
    pub shm_size: String,
    /// 模型最大上下文长度（0/空 = 使用模型默认）
    #[serde(default)]
    pub context_length: i64,
    /// 张量并行大小
    #[serde(default)]
    pub tensor_parallel_size: i64,
    /// 静态内存占比（0.0 = 自动）
    #[serde(default)]
    pub mem_fraction_static: f32,
    /// 模型权重/激活数据类型（auto/half/float16/bfloat16/float/float32）
    #[serde(default)]
    pub dtype: String,
    /// 量化方法（空 = 不指定，NVFP4 模型自动从 config 解析）
    #[serde(default)]
    pub quantization: String,
    /// KV cache 数据类型（auto/fp8_e5m2/fp8_e4m3/bf16/nvfp4/fp4_mx_block16）
    #[serde(default)]
    pub kv_cache_dtype: String,
    /// 调度策略（fcfs/lpm/random/dfs-weight/lof/priority）
    #[serde(default)]
    pub schedule_policy: String,
    /// 最大运行中请求数（0 = 自动）
    #[serde(default)]
    pub max_running_requests: i64,
    /// 最大排队请求数（0 = 自动）
    #[serde(default)]
    pub max_queued_requests: i64,
    /// chunked prefill 大小（0 = 自动，-1 = 禁用）
    #[serde(default)]
    pub chunked_prefill_size: i64,
    /// 日志级别（info/debug/warning/error/critical）
    #[serde(default)]
    pub log_level: String,
    /// 记录所有请求日志
    #[serde(default)]
    pub log_requests: bool,
    /// 启动 Prometheus metrics
    #[serde(default)]
    pub enable_metrics: bool,
    /// 推理模型 parser（deepseek-r1/qwen3 等，推理模型专用）
    #[serde(default)]
    pub reasoning_parser: String,
    /// 工具调用 parser（qwen/qwen25/deepseekv3 等）
    #[serde(default)]
    pub tool_call_parser: String,
    /// 额外参数（每行一个 `key=value`，原样拼成 --key value 追加到命令尾部）
    #[serde(default)]
    pub extra_args: String,
}

fn default_sglang_image() -> String {
    "lmsysorg/sglang:v0.5.17".to_string()
}

fn default_sglang_shm() -> String {
    "64g".to_string()
}

// ===== 自动更新相关结构 =====

#[derive(Serialize, Deserialize, Clone)]
pub struct PlatformUpdate {
    #[serde(rename = "appUrl")]
    pub app_url: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub windows: Option<PlatformUpdate>,
    #[serde(rename = "mac")]
    pub mac_os: Option<PlatformUpdate>,
    #[serde(rename = "linux")]
    pub linux: Option<PlatformUpdate>,
}

#[derive(Serialize, Clone)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub remote_version: String,
    pub current_version: String,
    pub download_url: Option<String>,
    pub changelog_url: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct PartFileProgress {
    pub model_id: String,
    pub existing_size: u64,
}

#[derive(Serialize, Clone)]
pub struct LocalModel {
    pub model_id: String,
    pub files: Vec<String>,
}
