#!/usr/bin/env bash
# ADM-BE · MiniMax-H3（ComfyUI 主线）镜像一键构建脚本 —— 支持在「另一台设备」构建后拷回 DGX Spark
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md §4.1
#
# 为什么单独给脚本：DGX Spark（ARM64）上常遇到 Docker Hub / github.com 不可达；本脚本把应用内
# 「构建镜像」的全部网络兜底逻辑搬到命令行，可在网络更好的设备上构建，再 docker save 拷回：
#   - 基础镜像源解析（**以 daemon 视角为准**：本地已有 → daemon 侧小镜像试拉官方 ref → pull 到本地再构建），
#     默认不内置任何第三方加速器（只用官方 ref，走 daemon 自身的全局代理 / daemon.json 配置）；
#     构建中若仍遇 registry 不可达，会按显式给出的候选源依次重试
#   - ComfyUI 源码源自动回退：官方 → gitee 镜像 → gitcode 镜像 → ghfast / gh-proxy 加速器
#   - 克隆结果 commit 校验（防第三方镜像被篡改 / 滞后）
#   - 构建期代理注入（本机代理自动加 --network=host；RUN 步骤 apt/pip/git 走代理）
#   - PyPI 源按吞吐自动择优（官方 / 清华 / 阿里 / 腾讯 / 华为云；torch 依赖不再回落到 files.pythonhosted.org）
#   - 强制 BuildKit：Dockerfile 三个 pip 层用 cache mount 缓存 wheel（失败重试不重下 2 GB+）
#   - 跨架构引导（目标默认 linux/arm64；x86 宿主需 binfmt/QEMU，会提示安装命令）
#   - 一键导出 tar.gz（docker save | gzip -1）+ sha256，拷到 DGX 后 docker load
#
# 典型流程：
#   1) 在（网络更好的）设备上： ./build-image.sh --save
#   2) 把生成的 *.tar.gz 拷到 DGX
#   3) 在 DGX 上：              gunzip -c adm-comfyui-h3_nvfp4-20260916-linux_arm64.tar.gz | docker load
#   4) 应用「视频生成」页 → 启动（engine_image 需与 --tag 一致，默认值已对齐 model.json）
#   注：权重不在此脚本范围（≈44–67 GB），仍由应用「下载权重」或 download-comfy-h3.sh 落盘到 DGX。
#
# 用法：
#   ./build-image.sh [选项]
#
# 选项：
#   -t, --tag <image:tag>   镜像名（默认 adm-comfyui-h3:nvfp4-20260916，须与 model.json engine_image 一致）
#       --ref <tag>         ComfyUI 版本（默认 v0.30.0；H3 需 ≥ 0.30.0）
#       --ref-sha <sha>     commit 校验值（默认内置 v0.30.0 上游 SHA；传空串 = 跳过校验）
#       --base <image>      强制基础镜像（跳过探测）
#       --proxy <url>       构建期代理（默认取 $HTTPS_PROXY / $https_proxy；本机代理自动 --network=host）
#       --pip-index <url>   强制 PyPI 源（默认按吞吐自动择优：官方 / 清华 / 阿里 / 腾讯 / 华为云）
#       --torch-index <url> PyTorch wheel 源（默认官方 cu130）
#       --platform <p>      目标平台（默认 linux/arm64，DGX Spark 用）
#       --save [路径]       构建后导出 tar.gz（路径可为文件或目录；默认当前目录）
#       --force             镜像已存在时也重建
#       --no-verify         跳过构建后的容器内自检（跑容器验证）
#       --skip-runtime-check 跳过镜像内构建期自检（跨架构 QEMU 下异常时用；到 DGX 实机再验）
#       --dry-run           只打印解析结果与将执行的命令，不构建
#   -h, --help              显示本帮助
set -euo pipefail

DEFAULT_TAG="adm-comfyui-h3:nvfp4-20260916"
DEFAULT_REF="v0.30.0"
# v0.30.0 上游 commit（已与 gitee/gitcode/ghfast/gh-proxy 四源核对一致）
DEFAULT_REF_SHA="b1693ecba9f5b65f8c80ab36b195ab963ec92413"
DEFAULT_PLATFORM="linux/arm64"
BASE_IMAGE="nvidia/cuda:13.0.0-runtime-ubuntu24.04"
BASE_PROBE_TIMEOUT=30
# 基础镜像源候选：默认**只用官方 ref**——走 daemon 自己的网络（系统/全局代理、daemon.json 的
# registry-mirrors / proxies）。**脚本不内置任何第三方加速器、也不会改你的 Docker 配置**；
# 确需加速器时自己显式给出（优先级：--base > ADM_BASE_MIRRORS）
ADM_BASE_MIRRORS="${ADM_BASE_MIRRORS:-}"
COMFYUI_SOURCES=(
  "https://github.com/comfyanonymous/ComfyUI"
  "https://gitee.com/mirrors/ComfyUI.git"
  "https://gitcode.com/gh_mirrors/co/ComfyUI.git"
  "https://ghfast.top/https://github.com/comfyanonymous/ComfyUI"
  "https://gh-proxy.com/https://github.com/comfyanonymous/ComfyUI"
)
# PyPI 源候选（官方 + 常见国内镜像）：torch 的依赖 / ComfyUI requirements / comfy-kitchen 都走选出的源。
# 官方源可达 ≠ 可下：未托管在 PyTorch 索引的包会回落到 files.pythonhosted.org，弱网下长时间停读。
PYPI_SOURCES=(
  "https://pypi.org/simple"
  "https://pypi.tuna.tsinghua.edu.cn/simple"
  "https://mirrors.aliyun.com/pypi/simple"
  "https://mirrors.cloud.tencent.com/pypi/simple"
  "https://repo.huaweicloud.com/repository/pypi/simple"
)

SELF="$0"
if [ -n "${BASH_SOURCE:-}" ]; then
  SELF="${BASH_SOURCE[0]}"
fi
SCRIPT_DIR="$(cd "$(dirname "$SELF")" && pwd)"

log() { printf '[build-image] %s\n' "$*"; }
logerr() { printf '[build-image] %s\n' "$*" >&2; } # 供被 $( ) 捕获的函数内部使用
warn() { printf '[build-image] WARN: %s\n' "$*" >&2; }
die() {
  printf '[build-image] ERROR: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
用法：build-image.sh [选项]

  -t, --tag <image:tag>   镜像名（默认 adm-comfyui-h3:nvfp4-20260916）
      --base <image>      强制基础镜像（跳过探测；仍会尝试先 pull 到本地）
      --ref <tag>         ComfyUI 版本（默认 v0.30.0）
      --ref-sha <sha>     commit 校验值（默认内置 v0.30.0 上游 SHA；传空串 = 跳过校验）
      --proxy <url>       构建期代理（默认取 $HTTPS_PROXY / $https_proxy）
      --pip-index <url>   强制 PyPI 源（默认按吞吐自动择优：官方 / 清华 / 阿里 / 腾讯 / 华为云）
      --torch-index <url> PyTorch wheel 源（默认官方 cu130）
      --platform <p>      目标平台（默认 linux/arm64）
      --save [路径]       构建后导出 tar.gz（默认当前目录）
      --force             镜像已存在时也重建
      --no-verify         跳过构建后的容器内自检
      --skip-runtime-check 跳过镜像内构建期自检（QEMU 跨架构异常时用）
      --dry-run           只打印不执行
  -h, --help              显示本帮助

示例：
  ./build-image.sh --save
  ./build-image.sh --proxy http://127.0.0.1:1080 --pip-index https://pypi.tuna.tsinghua.edu.cn/simple
  ./build-image.sh --dry-run

说明：基础镜像以 **daemon 视角** 判定可达（小镜像试拉；客户端代理不算数），
      选定后先 pull 到本地再 docker build——避免「探测说官方可达、构建却卡在
      registry-1.docker.io i/o timeout」。默认只用官方 ref，走 daemon 自己的网络
      （全局代理 / daemon.json 的 registry-mirrors、proxies）；脚本不会改你的 Docker 配置，
      也不内置加速器——确需时用 --base <你的加速器前缀>/<ref> 或环境变量：
        ADM_BASE_MIRRORS="host1 host2" ./build-image.sh
USAGE
}

# ---------- 参数 ----------
TAG="$DEFAULT_TAG"
OPT_REF=""
OPT_REF_SHA=""
OPT_BASE=""
OPT_PROXY="${HTTPS_PROXY:-${https_proxy:-}}"
OPT_PIP_INDEX=""
OPT_TORCH_INDEX=""
OPT_PLATFORM=""
SAVE_TO=""
FORCE=0
DO_VERIFY=1
SKIP_RUNTIME_CHECK=0
DRY_RUN=0

while [ $# -gt 0 ]; do
  case "$1" in
    -t | --tag)
      [ $# -ge 2 ] || die "--tag 需要一个值"
      TAG="$2"
      shift 2
      ;;
    --ref)
      [ $# -ge 2 ] || die "--ref 需要一个值"
      OPT_REF="$2"
      shift 2
      ;;
    --ref-sha)
      [ $# -ge 2 ] || die "--ref-sha 需要一个值（可传空串跳过校验）"
      OPT_REF_SHA="$2"
      shift 2
      ;;
    --base)
      [ $# -ge 2 ] || die "--base 需要一个值"
      OPT_BASE="$2"
      shift 2
      ;;
    --proxy)
      [ $# -ge 2 ] || die "--proxy 需要一个值"
      OPT_PROXY="$2"
      shift 2
      ;;
    --pip-index)
      [ $# -ge 2 ] || die "--pip-index 需要一个值"
      OPT_PIP_INDEX="$2"
      shift 2
      ;;
    --torch-index)
      [ $# -ge 2 ] || die "--torch-index 需要一个值"
      OPT_TORCH_INDEX="$2"
      shift 2
      ;;
    --platform)
      [ $# -ge 2 ] || die "--platform 需要一个值"
      OPT_PLATFORM="$2"
      shift 2
      ;;
    --save)
      shift
      if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then
        SAVE_TO="$1"
        shift
      else
        SAVE_TO="."
      fi
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --no-verify)
      DO_VERIFY=0
      shift
      ;;
    --skip-runtime-check)
      SKIP_RUNTIME_CHECK=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "未知参数：$1（--help 查看用法）"
      ;;
  esac
done

REF="${OPT_REF:-$DEFAULT_REF}"
PLATFORM="${OPT_PLATFORM:-$DEFAULT_PLATFORM}"
if [ -z "$OPT_REF_SHA" ] && [ "$REF" = "$DEFAULT_REF" ]; then
  OPT_REF_SHA="$DEFAULT_REF_SHA"
fi

[ -f "$SCRIPT_DIR/Dockerfile" ] || die "找不到 Dockerfile：$SCRIPT_DIR/Dockerfile"

# ---------- 架构 ----------
host_arch() {
  case "$(uname -m 2>/dev/null || printf unknown)" in
    aarch64 | arm64) printf 'arm64' ;;
    x86_64 | amd64) printf 'amd64' ;;
    *) printf 'unknown' ;;
  esac
}

target_arch() {
  case "$1" in
    *arm64* | *aarch64*) printf 'arm64' ;;
    *amd64* | *x86_64*) printf 'amd64' ;;
    *) host_arch ;;
  esac
}

HOST_ARCH="$(host_arch)"
TARGET_ARCH="$(target_arch "$PLATFORM")"

# ---------- 预检 ----------
if [ "$DRY_RUN" -eq 0 ]; then
  command -v docker >/dev/null 2>&1 || die "未找到 docker CLI（请先安装 Docker / 或用 --dry-run 只看计划）"
  docker info >/dev/null 2>&1 || die "Docker daemon 不可用（docker info 失败）"
  # 本 Dockerfile 用了 --mount=type=cache（pip wheel 缓存，失败重试不重下 2 GB+）→ 必须 BuildKit：
  # Docker 23+ 默认即 BuildKit；更老引擎（Ubuntu 22.04 的 docker.io 20.10 等）需显式 DOCKER_BUILDKIT=1
  if [ "${DOCKER_BUILDKIT:-1}" = "0" ]; then
    warn "检测到 DOCKER_BUILDKIT=0（classic builder）：Dockerfile 的 --mount=type=cache 需要 BuildKit，已强制启用"
  fi
  export DOCKER_BUILDKIT=1
  SERVER_VERSION="$(docker version --format '{{.Server.Version}}' 2>/dev/null || true)"
  SERVER_MAJOR="${SERVER_VERSION%%.*}"
  case "$SERVER_MAJOR" in
    '' | *[!0-9]*) ;;
    *)
      if [ "$SERVER_MAJOR" -lt 23 ]; then
        log "Docker 引擎 ${SERVER_VERSION}（< 23）：已显式启用 BuildKit（DOCKER_BUILDKIT=1）"
      fi
      ;;
  esac
  # daemon 的代理 / 镜像加速（BuildKit 解析 FROM 只认这些，客户端代理不算数）
  DAEMON_NET_HINT="$(docker info 2>/dev/null | grep -iE '^ *(HTTP|HTTPS) Proxy:|^ *Registry Mirrors:' | tr -d '\r' | tr '\n' ' ' || true)"
  if [ -n "$DAEMON_NET_HINT" ]; then
    log "daemon 网络配置：${DAEMON_NET_HINT}（本脚本不会改动它）"
  else
    logerr "daemon 未显示代理 / 镜像加速配置（docker info）——若 daemon 拉不到镜像，请让 daemon 自身走上你的全局代理（systemd 环境变量 HTTP_PROXY/HTTPS_PROXY）或写 daemon.json proxies；本脚本不会替你修改 Docker 配置"
  fi
fi

is_docker_desktop() {
  command -v docker >/dev/null 2>&1 || return 1
  docker info --format '{{.OperatingSystem}}' 2>/dev/null | grep -qi 'docker desktop'
}

# ---------- 基础镜像源解析（以 **daemon 视角** 为准） ----------
# 教训（2026-09-16 实测）：`docker manifest inspect` 是 **CLI 侧** 请求，会吃客户端代理
# （HTTPS_PROXY / --proxy），而 BuildKit 解析 FROM 由 **daemon** 发起（只认 daemon.json 的
# proxies，registry-mirrors 也只作用于 daemon 的 pull）——于是会出现
# 「探测说官方源可达 → 构建卡在 registry-1.docker.io: i/o timeout」。
# 现在的做法：
#   ① 本地已有候选镜像 → 直接用（零网络）；
#   ② 探针改为 **daemon 侧小镜像试拉**（hello-world，与候选同 registry，~10 KB，超时 BASE_PROBE_TIMEOUT）；
#   ③ 选定后先 `docker pull --platform <目标平台> <ref>` 把基础镜像落到本地：BuildKit 解析 FROM
#      直接命中本地镜像、不再回源（跨架构时也拉对应架构变体）。
# 只有 GNU/coreutils 的 timeout 才支持「超时时间 + 命令」；Windows(Git Bash)/macOS 无此命令时退回不带超时
HAVE_TIMEOUT=0
if command -v timeout >/dev/null 2>&1 && timeout --version >/dev/null 2>&1; then
  HAVE_TIMEOUT=1
fi

# 同 registry 的小镜像探针（镜像加速器有 <host>/<img> 与 <host>/library/<img> 两种形态，都试）
probe_refs_of() {
  case "$1" in
    "$BASE_IMAGE") printf '%s\n' "hello-world:latest" ;;
    *) printf '%s\n%s\n' "${1%/*}/hello-world:latest" "${1%/*}/library/hello-world:latest" ;;
  esac
}

daemon_can_pull() { # 0 = daemon 侧（含 daemon.json 的 mirror/proxy）确实能拉到该源
  for img in $(probe_refs_of "$1"); do
    if [ "$HAVE_TIMEOUT" -eq 1 ]; then
      timeout "$BASE_PROBE_TIMEOUT" docker pull -q --platform "$PLATFORM" "$img" >/dev/null 2>&1 && return 0
    else
      docker pull -q --platform "$PLATFORM" "$img" >/dev/null 2>&1 && return 0
    fi
  done
  return 1
}

image_local() { docker image inspect "$1" >/dev/null 2>&1; }

BASE_CANDIDATES=("$BASE_IMAGE")
if [ -n "$ADM_BASE_MIRRORS" ]; then
  for mirror in $(printf '%s' "$ADM_BASE_MIRRORS" | tr ',' ' '); do
    if [ -n "$mirror" ]; then BASE_CANDIDATES+=("${mirror%/}/${BASE_IMAGE}"); fi
  done
fi

BASE_TRY_ORDER=()
append_base() { case " ${BASE_TRY_ORDER[*]:-} " in *" $1 "*) return 0 ;; esac; BASE_TRY_ORDER+=("$1"); }

if [ -n "$OPT_BASE" ]; then
  append_base "$OPT_BASE"
  log "基础镜像：${OPT_BASE}（手动指定）"
else
  for cand in "${BASE_CANDIDATES[@]}"; do
    if image_local "$cand"; then
      append_base "$cand"
      logerr "  本地已有：${cand}"
    fi
  done
  logerr "探测基础镜像源（daemon 视角，小镜像试拉；与 BuildKit 同一条网络）..."
  for cand in "${BASE_CANDIDATES[@]}"; do
    case " ${BASE_TRY_ORDER[*]:-} " in *" $cand "*) continue ;; esac
    if daemon_can_pull "$cand"; then
      append_base "$cand"
      logerr "  daemon 可达：${cand}"
    else
      logerr "  跳过（daemon 不可达）：${cand}"
    fi
  done
  # 兜底：全部探测失败时也保留「官方 → 加速器」顺序，构建阶段会自动换源重试
  for cand in "${BASE_CANDIDATES[@]}"; do append_base "$cand"; done
fi
RESOLVED_BASE="${BASE_TRY_ORDER[0]:-$BASE_IMAGE}"
log "基础镜像候选顺序：${BASE_TRY_ORDER[*]}"

# ---------- ComfyUI 源码源探测（可达的排前面） ----------
probe_git_repo() {
  command -v curl >/dev/null 2>&1 || return 2
  curl -fsS -m 8 -o /dev/null "$1/info/refs?service=git-upload-pack" >/dev/null 2>&1
}

reachable=""
others=""
if command -v curl >/dev/null 2>&1; then
  logerr "探测 ComfyUI 源码源..."
  for src in "${COMFYUI_SOURCES[@]}"; do
    if probe_git_repo "$src"; then
      reachable="${reachable}${reachable:+|}$src"
      logerr "  可达：${src}"
    else
      others="${others}${others:+|}$src"
    fi
  done
fi
ordered="$reachable"
if [ -n "$others" ]; then
  if [ -n "$ordered" ]; then
    ordered="${ordered}|${others}"
  else
    ordered="$others"
  fi
fi
if [ -z "$ordered" ]; then
  ordered="${COMFYUI_SOURCES[0]}"
fi
CHOSEN_REPO="${ordered%%|*}"
if [ "$ordered" = "$CHOSEN_REPO" ]; then
  CHOSEN_FALLBACKS=""
else
  CHOSEN_FALLBACKS="${ordered#*|}"
fi
FALLBACK_COUNT=0
if [ -n "$CHOSEN_FALLBACKS" ]; then
  FALLBACK_COUNT="$(printf '%s' "$CHOSEN_FALLBACKS" | tr '|' '\n' | grep -c .)"
fi
log "ComfyUI 源码源：${CHOSEN_REPO}（候选回退 ${FALLBACK_COUNT} 个）"

# ---------- PyPI 源按吞吐择优（torch 依赖 / requirements / comfy-kitchen 都走它） ----------
# 官方源可达 ≠ 可下：未托管在 PyTorch 索引的包会回落到 files.pythonhosted.org
# （实测 8.7 KB/s → pip Read timed out，整层构建失败），故按实测吞吐而非可达性选源。
probe_pypi_speed() {
  command -v curl >/dev/null 2>&1 || return 2
  curl -fsS -m 8 -o /dev/null -w '%{speed_download}' "$1/pip/" 2>/dev/null
}

PIP_INDEX_CHOSEN="$OPT_PIP_INDEX"
if [ -n "$PIP_INDEX_CHOSEN" ]; then
  log "PyPI 源（--pip-index 指定）：${PIP_INDEX_CHOSEN}"
else
  if command -v curl >/dev/null 2>&1; then
    logerr "探测 PyPI 源（按吞吐择优）..."
    best_bps=0
    for src in "${PYPI_SOURCES[@]}"; do
      bps="$(probe_pypi_speed "$src" || true)"
      case "$bps" in '' | *[!0-9.]* | 0 | 0.0) continue ;; esac
      bps_int="${bps%%.*}"
      [ -n "$bps_int" ] || continue
      logerr "  实测 $((bps_int / 1024)) KB/s  ${src}"
      if [ "$bps_int" -gt "$best_bps" ]; then
        best_bps="$bps_int"
        PIP_INDEX_CHOSEN="$src"
      fi
    done
  fi
  if [ -n "$PIP_INDEX_CHOSEN" ] && [ "$PIP_INDEX_CHOSEN" = "${PYPI_SOURCES[0]}" ]; then
    log "PyPI 源：${PIP_INDEX_CHOSEN}（官方源实测最优）"
  elif [ -n "$PIP_INDEX_CHOSEN" ]; then
    log "PyPI 源改用镜像：${PIP_INDEX_CHOSEN}"
  else
    warn "未探测到可达的 PyPI 源，仍按官方源构建（Dockerfile 内自带官方兜底与超时重试）"
  fi
fi

# ---------- 依赖检查 ----------
if [ -n "$OPT_TORCH_INDEX" ]; then
  TORCH_INDEX="$OPT_TORCH_INDEX"
else
  TORCH_INDEX="https://download.pytorch.org/whl/cu130"
fi

# ---------- 构建参数 ----------
build_args=()
if [ -n "$PIP_INDEX_CHOSEN" ]; then
  build_args+=(--build-arg "PIP_INDEX_URL=${PIP_INDEX_CHOSEN}")
fi
build_args+=(--build-arg "TORCH_INDEX_URL=${TORCH_INDEX}")
build_args+=(--build-arg "COMFYUI_REF=${REF}")
build_args+=(--build-arg "COMFYUI_REPO=${CHOSEN_REPO}")
if [ -n "$CHOSEN_FALLBACKS" ]; then
  build_args+=(--build-arg "COMFYUI_REPO_FALLBACKS=$(printf '%s' "$CHOSEN_FALLBACKS" | tr '|' ' ')")
fi
if [ -n "$OPT_REF_SHA" ]; then
  build_args+=(--build-arg "COMFYUI_REF_SHA=${OPT_REF_SHA}")
fi
if [ "$SKIP_RUNTIME_CHECK" -eq 1 ]; then
  build_args+=(--build-arg "SKIP_RUNTIME_CHECK=1")
fi

platform_args=()
if [ "$TARGET_ARCH" != "$HOST_ARCH" ]; then
  platform_args+=(--platform "$PLATFORM")
  warn "跨架构构建（${HOST_ARCH} → ${TARGET_ARCH}）：需 binfmt/QEMU 支持，apt/pip 在模拟下明显变慢（预计 1–3 小时）"
  BINFMT_ARCH="$TARGET_ARCH"
  case "$TARGET_ARCH" in
    arm64) BINFMT_ARCH="aarch64" ;;
    amd64) BINFMT_ARCH="x86_64" ;;
  esac
  if [ -d /proc/sys/fs/binfmt_misc ] && ! is_docker_desktop; then
    if ls /proc/sys/fs/binfmt_misc 2>/dev/null | grep -q "qemu-${BINFMT_ARCH}"; then
      log "已检测到 binfmt 处理器 qemu-${BINFMT_ARCH}：跨架构可构建"
    else
      warn "未检测到 binfmt 处理器 qemu-${BINFMT_ARCH}，先执行一次： sudo docker run --privileged --rm tonistiigi/binfmt --install ${TARGET_ARCH}"
    fi
  else
    warn "若报 exec format error，先执行一次： docker run --privileged --rm tonistiigi/binfmt --install ${TARGET_ARCH}"
  fi
  warn "QEMU 下若镜像内自检（torch/comfy_kitchen）异常，可加 --skip-runtime-check 跳过，到 DGX 实机再验"
fi

proxy_args=()
if [ -n "$OPT_PROXY" ]; then
  case "$OPT_PROXY" in
    *localhost* | *127.0.0.1* | *::1*)
      if is_docker_desktop; then
        warn "检测到 Docker Desktop：容器内 127.0.0.1 指向 VM 自身；若代理跑在宿主，请改用 http://host.docker.internal:<port>"
      else
        proxy_args+=(--network=host)
        log "代理在本机：构建期使用 host 网络（--network=host）"
      fi
      ;;
  esac
  for key in HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY http_proxy https_proxy all_proxy no_proxy; do
    value="$OPT_PROXY"
    case "$key" in
      [Nn][Oo]_*) value="localhost,127.0.0.0/8,::1" ;;
    esac
    build_args+=(--build-arg "${key}=${value}")
  done
  log "构建期注入代理：${OPT_PROXY}（只影响 RUN 步骤 apt/pip/git）——注意：FROM 解析由 daemon 发起，客户端代理不算数（脚本已改为先把基础镜像 pull 到本地再构建；要 daemon 也走代理请写 daemon.json proxies）"
fi

# BASE 不进 build_args：每个候选基础镜像重试时单独拼（见下）
make_build_cmd() {
  BUILD_CMD=(docker build -t "$TAG")
  if [ "${DOCKER_BUILDKIT:-1}" != "0" ]; then
    BUILD_CMD+=(--progress=plain)
  fi
  BUILD_CMD+=("${platform_args[@]}" "${proxy_args[@]}" "${build_args[@]}" --build-arg "BASE=$1" "$SCRIPT_DIR")
}

# ---------- 已存在则跳过 ----------
SKIP_BUILD=0
if [ "$DRY_RUN" -eq 0 ] && [ "$FORCE" -eq 0 ]; then
  if docker image inspect "$TAG" >/dev/null 2>&1; then
    SKIP_BUILD=1
    log "镜像 ${TAG} 已存在本地，跳过构建（--force 可强制重建）"
  fi
fi

# ---------- 执行 ----------
if [ "$DRY_RUN" -eq 1 ]; then
  log "dry-run：解析结果"
  log "  镜像 tag   ：${TAG}"
  log "  目标平台   ：${PLATFORM}（宿主 ${HOST_ARCH}）"
  log "  基础镜像   ：${RESOLVED_BASE}（候选顺序：${BASE_TRY_ORDER[*]}）"
  log "  PyPI 源   ：${PIP_INDEX_CHOSEN:-官方（未探测到更快源）}"
  log "  源码源     ：${CHOSEN_REPO}"
  log "  commit 校验：${OPT_REF_SHA:-（跳过）}"
  if [ "$SKIP_RUNTIME_CHECK" -eq 1 ]; then
    log "  镜像内自检 ：跳过（SKIP_RUNTIME_CHECK=1）"
  fi
  log "  构建上下文 ：${SCRIPT_DIR}"
  log "  将执行     ："
  make_build_cmd "$RESOLVED_BASE"
  printf '    %s\n' "${BUILD_CMD[*]}"
  if [ -n "$SAVE_TO" ]; then
    log "  导出计划   ：docker save ${TAG} | gzip -1 > <导出路径>"
  fi
  exit 0
fi

if [ "$SKIP_BUILD" -eq 0 ]; then
  log "开始构建：${TAG}（上下文 ${SCRIPT_DIR}）"
  BUILD_LOG="${TMPDIR:-/tmp}/adm-comfyui-build.$$.log"
  built=0
  tried=""
  for base in "${BASE_TRY_ORDER[@]}"; do
    case " $tried " in *" $base "*) continue ;; esac
    tried="$tried $base"
    # 基础镜像先落到本地：BuildKit 解析 FROM 命中本地，不再回源（daemon 不可达时才需要换源重试）
    if image_local "$base"; then
      log "基础镜像已在本地：${base}"
    elif docker pull --platform "$PLATFORM" "$base"; then
      log "基础镜像已就绪：${base}"
    else
      warn "基础镜像拉取失败，尝试下一个源：${base}"
      continue
    fi
    make_build_cmd "$base"
    log "docker build（BASE=${base}）"
    if "${BUILD_CMD[@]}" 2>&1 | tee "$BUILD_LOG"; then
      built=1
      break
    fi
    if grep -qiE 'failed to resolve source metadata|failed to load metadata|failed to do request.*manifest|dial tcp' "$BUILD_LOG"; then
      warn "基础镜像 ${base} 在 daemon/BuildKit 侧不可达（见上方输出），自动换源重试"
      continue
    fi
    die "构建失败（详见上方输出，完整日志 ${BUILD_LOG}；重试：--force）"
  done
  if [ "$built" != "1" ]; then
    die "所有候选基础镜像均不可用：${tried}\n  排查：① 让 daemon 自身能访问该 registry（全局代理 / daemon.json proxies，脚本不代改配置）；② 显式指定可达镜像源：--base <你的加速器前缀>/${BASE_IMAGE} 或 ADM_BASE_MIRRORS=\"host\"；③ 在其它机器 docker save/load 基础镜像到本机（脚本会优先用本地已有镜像，零网络）"
  fi
  rm -f "$BUILD_LOG"
fi

# ---------- 构建后自检（可选） ----------
if [ "$DO_VERIFY" -eq 1 ]; then
  log "自检：容器内确认 triton JIT 工具链（gcc / Python.h）+ 导入 torch / comfy_kitchen ..."
  if ! docker run --rm --entrypoint bash "$TAG" -c "command -v gcc >/dev/null || { echo '缺少 gcc：triton JIT 无法编译 cuda_utils（H3 文本编码器会报 Failed to find C compiler）' >&2; exit 1; }; python3 -c \"import os,sysconfig; hdr=os.path.join(sysconfig.get_paths()['include'],'Python.h'); assert os.path.exists(hdr), '缺少 Python.h（python3-dev）：'+hdr; print('triton JIT 工具链 OK：gcc +', hdr)\""; then
    warn "自检未通过：triton JIT 工具链缺失（见上面输出）——检查 Dockerfile 的 apt 步骤是否装了 gcc / libc6-dev / python3-dev"
  elif ! docker run --rm --entrypoint python3 "$TAG" -c "import importlib.util as u, torch; print('torch', torch.__version__, '| comfy-kitchen:', 'ok' if u.find_spec('comfy_kitchen') else 'missing')"; then
    warn "自检未通过：请查看上面输出（跨架构构建可能受 QEMU 影响，可加 --no-verify 跳过）"
  else
    log "自检通过（GPU 相关检查请到 DGX 实机做）"
  fi
fi

# ---------- 导出 tar.gz ----------
if [ -n "$SAVE_TO" ]; then
  safe_name="$(printf '%s' "$TAG" | tr ':/' '__')"
  out="$SAVE_TO"
  case "$out" in
    . | ./) out="./${safe_name}-${TARGET_ARCH}.tar.gz" ;;
  esac
  if [ -d "$out" ]; then
    out="$out/${safe_name}-${TARGET_ARCH}.tar.gz"
  fi
  log "导出镜像：docker save ${TAG} | gzip -1 > ${out}（体积约 8–10 GB，请确认磁盘空间）"
  docker save "$TAG" | gzip -1 >"$out"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$out"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$out"
  fi
  log "拷到 DGX 后导入： gunzip -c $(basename "$out") | docker load"
fi

log "完成：${TAG}"
log "下一步：确认 DGX 上 model.json 的 engine_image = ${TAG}，再在「视频生成」页启动；权重用页内「下载权重」或 download-comfy-h3.sh 落盘"
