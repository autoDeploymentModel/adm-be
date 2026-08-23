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
    /// 模型指定的 SGLang 镜像（完整 Docker 镜像名，如 "lmsysorg/sglang:dev-cu13-qwen38-27b-dflash2"）；非空时启动直接使用
    #[serde(default)]
    pub sglang_version: String,
    /// 模型指定的 SGLang 启动参数（官方 cookbook 推荐值，如 ["--mem-fraction-static 0.80"]）；
    /// 每条为完整 `--key value` 或 `--flag`，最后追加，优先级最高（可覆盖设置页同名参数与默认值）
    #[serde(default)]
    pub sglang_flags: Vec<String>,
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
    /// 多机互联配置（DGX Spark 集群，2+ 节点）；enabled=false 时走单机路径
    #[serde(default)]
    pub multi_node_args: MultiNodeArgs,
    /// 调试模式：开启后在软件根目录记录 API/SSE 交互日志（每次重启自动清空）
    #[serde(default)]
    pub debug_logging: bool,
    /// 界面语言（"zh" 中文 / "en" English，空或未知回退中文）
    #[serde(default)]
    pub language: String,
}

/// 多机互联配置（DGX Spark 集群）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MultiNodeArgs {
    /// 总开关：false = 走现有单机代码路径
    #[serde(default)]
    pub enabled: bool,
    /// 节点清单（按下标即 rank；[0] 必须是本机 is_self=true）
    #[serde(default)]
    pub nodes: Vec<NodeInfo>,
    /// 分布式引导端口（--dist-init-addr 端口，默认 6464 与 SGLang 官方一致，不得与服务端口冲突）
    #[serde(default = "default_dist_init_port")]
    pub dist_init_port: u16,
    /// NCCL 通信端口（0 = 自动）
    #[serde(default)]
    pub nccl_port: u16,
    /// 互连网卡（NCCL_SOCKET_IFNAME / GLOO_SOCKET_IFNAME，空 = 自动）
    #[serde(default)]
    pub iface: String,
    /// 启用 RoCE（追加 --device /dev/infiniband、--ulimit memlock=-1:-1、--cap-add IPC_LOCK）
    #[serde(default = "default_true")]
    pub use_roce: bool,
    /// SSH 私钥路径（-i 指定；空 = 使用 ssh-agent / 默认 key）
    #[serde(default)]
    pub ssh_key_path: String,
    /// 额外容器环境变量（每行 KEY=VALUE，注入 docker run -e KEY=VALUE；如 NCCL_DEBUG=TRACE / NCCL_SOCKET_NTHREADS=1）
    #[serde(default)]
    pub extra_env: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NodeInfo {
    pub ip: String,
    pub ssh_user: String,
    /// SSH 端口（默认 22）
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    /// 本机标记（仅 nodes[0] 可为 true）
    #[serde(default)]
    pub is_self: bool,
    /// 该节点本地的模型目录（须已下载好模型）
    #[serde(default)]
    pub model_dir: String,
}

fn default_dist_init_port() -> u16 {
    6464
}

fn default_true() -> bool {
    true
}

fn default_ssh_port() -> u16 {
    22
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
