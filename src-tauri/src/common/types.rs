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
    /// vLLM Docker 镜像（完整名，如 "eugr/spark-vllm-b12x:latest"）；由远程 model.json 唯一指定
    #[serde(default)]
    pub vllm_image: String,
    /// vLLM 启动参数（如 ["--max-model-len 262144"]）；每条为完整 `--key value` 或 `--flag`，远程覆盖本地同名参数
    #[serde(default)]
    pub vllm_flags: Vec<String>,
    /// vLLM 容器环境变量（如 ["VLLM_USE_AOT_COMPILE=1"]）；每条 `KEY=VALUE`，注入 docker `-e`，与 vllm_flags 同级（模型清单优先级最高）
    #[serde(default)]
    pub vllm_env: Vec<String>,
    /// 附加挂载的模型目录 model_id 列表（如投机解码 drafter）；启动时逐条追加
    /// `-v <models>/<id>:/models/<id>:ro`（多机 worker 为远端 `<model_root>/<id>`），
    /// 目录不存在直接报「请先下载/同步」。缺省/空数组 = 不追加任何挂载，其他模型零影响。
    #[serde(default)]
    pub vllm_extra_mounts: Vec<String>,
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
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    pub launch_params: LaunchParams,
    /// vLLM 详细参数配置（Docker 部署用）
    #[serde(default)]
    pub vllm_args: VllmArgs,
    /// 多机互联配置（DGX Spark 集群，2+ 节点）；enabled=false 时走单机路径
    #[serde(default)]
    pub multi_node_args: MultiNodeArgs,
    /// 调试模式：开启后在软件根目录记录 API/SSE 交互日志（每次重启自动清空）
    #[serde(default)]
    pub debug_logging: bool,
    /// 界面语言（"zh" 中文 / "en" English，空或未知回退中文）
    #[serde(default)]
    pub language: String,
    /// 本地代理地址（模型文件 / Docker 镜像下载用，如 "http://127.0.0.1:1080"；空 = 直连）
    #[serde(default)]
    pub proxy_url: String,
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
    /// Ray 分布式引导端口（默认 6379，与 Ray 官方一致，不得与服务端口冲突）
    #[serde(default = "default_dist_init_port")]
    pub dist_init_port: u16,
    /// 互连网卡（NCCL_SOCKET_IFNAME / GLOO_SOCKET_IFNAME，空 = 自动）
    #[serde(default)]
    pub iface: String,
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
    6379
}
fn default_ssh_port() -> u16 {
    22
}

/// vLLM serve 详细参数（设置页可调，启动时拼成 `--key value`）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VllmArgs {
    /// docker --shm-size
    #[serde(default = "default_vllm_shm")]
    pub shm_size: String,
    /// 张量并行大小
    #[serde(default)]
    pub tensor_parallel_size: i64,
    /// GPU 显存利用率（0.0-1.0，0 = 自动）
    #[serde(default)]
    pub gpu_memory_utilization: f32,
    /// 量化方法（空 = 不指定，自动从 config 解析）
    #[serde(default)]
    pub quantization: String,
    /// KV cache 数据类型（auto/fp8/fp8_e4m3/bf16）
    #[serde(default)]
    pub kv_cache_dtype: String,
    /// 分布式执行后端（ray/mp，空 = 单机）
    #[serde(default)]
    pub distributed_executor_backend: String,
    /// 加载格式（safetensors/instanttensor/auto；空 = 不指定，使用 vLLM 默认）
    #[serde(default)]
    pub load_format: String,
    /// Block size（DeepSeek V4 推荐 256）
    #[serde(default = "default_vllm_block_size")]
    pub block_size: i64,
    /// Tokenizer 模式（deepseek_v4 等）
    #[serde(default)]
    pub tokenizer_mode: String,
    /// 工具调用 parser
    #[serde(default)]
    pub tool_call_parser: String,
    /// 推理 parser
    #[serde(default)]
    pub reasoning_parser: String,
    /// 自动工具选择
    #[serde(default = "default_true")]
    pub enable_auto_tool_choice: bool,
    /// 信任远程代码
    #[serde(default = "default_true")]
    pub trust_remote_code: bool,
    /// 最大并发序列数（0 = 自动）
    #[serde(default)]
    pub max_num_seqs: i64,
    /// 单批最大 Token 数（0 = 自动，-1 = 禁用）
    #[serde(default)]
    pub max_num_batched_tokens: i64,
    /// 额外参数（每行一个 `key=value`，原样拼成 --key value 追加到命令尾部）
    #[serde(default)]
    pub extra_args: String,
    /// 额外环境变量（每行一个 `KEY=VALUE`，注入 docker run -e KEY=VALUE）
    #[serde(default)]
    pub extra_env: String,
}

fn default_vllm_shm() -> String {
    "64g".to_string()
}

fn default_vllm_block_size() -> i64 {
    256
}

fn default_true() -> bool {
    true
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
    pub has_done: bool,
}
