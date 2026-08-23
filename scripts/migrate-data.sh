#!/usr/bin/env bash
# migrate-data.sh — ADM -> ADM-BE 一次性数据迁移（仅 Ubuntu / Linux）
# 新版 identifier 改为 com.adm.be 后，Linux 数据目录从旧 identifier 路径
#   $XDG_DATA_HOME/com.adm.admapp   （默认 ~/.local/share/com.adm.admapp）
# 迁移到
#   $XDG_DATA_HOME/com.adm.be       （默认 ~/.local/share/com.adm.be）
#
# 迁移源目录下全部顶层条目（models/ 已下载模型、sd/ 文生图、config.json 设置、
# .part 断点续传临时文件等）。同分区 mv 瞬时完成，源数据不再保留。
# Windows 版无需迁移（数据在各应用 exe 目录，互不共享），本脚本仅用于 Linux。
#
# 用法:
#   ./scripts/migrate-data.sh --dry-run      只预览，不执行
#   ./scripts/migrate-data.sh                确认后执行
#   ./scripts/migrate-data.sh --close-old    自动关闭旧版 ADM 进程后执行

set -euo pipefail

DRY_RUN=false
CLOSE_OLD=false
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY_RUN=true ;;
    --close-old) CLOSE_OLD=true ;;
    --help|-h)
      sed -n '2,16p' "$0"
      exit 0 ;;
    *) echo "未知参数: $arg（可用 --dry-run / --close-old）" >&2; exit 1 ;;
  esac
done

BASE="${XDG_DATA_HOME:-$HOME/.local/share}"
SRC="$BASE/com.adm.admapp"
DST="$BASE/com.adm.be"
LOG="$DST/migrate-data.log"

if [ ! -d "$SRC" ]; then
  echo "未发现旧数据目录 $SRC （可能已迁移过），无需迁移。"
  exit 0
fi
if [ "$SRC" = "$DST" ]; then
  echo "错误：源与目标目录相同。" >&2
  exit 1
fi

# ---------- 进程检查 ----------
NEW_PROCS=""
OLD_PROCS=""
if command -v pgrep >/dev/null 2>&1; then
  for n in ADM-BE adm-be; do
    pgrep -x "$n" >/dev/null 2>&1 && NEW_PROCS="$NEW_PROCS $n"
  done
  for n in ADM adm admAgent; do
    pgrep -x "$n" >/dev/null 2>&1 && OLD_PROCS="$OLD_PROCS $n"
  done
fi
if [ -n "$NEW_PROCS" ]; then
  echo "检测到新版 ADM-BE 正在运行：$NEW_PROCS，请先退出后再迁移。" >&2
  exit 1
fi
if [ -n "$OLD_PROCS" ] && [ "$CLOSE_OLD" != true ]; then
  echo "检测到旧版 ADM 进程仍在运行：$OLD_PROCS，迁移会移走其数据。" >&2
  echo "请先退出旧版；或使用 --close-old 自动关闭后继续。" >&2
  exit 1
fi
if [ -n "$OLD_PROCS" ] && [ "$CLOSE_OLD" = true ]; then
  for n in $OLD_PROCS; do
    pkill -x "$n" >/dev/null 2>&1 || true
  done
  sleep 2
  echo "已关闭旧版 ADM 进程：$OLD_PROCS"
fi

# ---------- 列出迁移条目 ----------
shopt -s nullglob dotglob
ENTRIES=( "$SRC"/* )
shopt -u dotglob

if [ ${#ENTRIES[@]} -eq 0 ]; then
  echo "旧数据目录 $SRC 为空，无需迁移。"
  rmdir "$SRC"
  exit 0
fi

echo ""
echo "===== ADM -> ADM-BE 一次性数据迁移（Linux）====="
printf '%-20s %14s  %s\n' "项目" "大小" "目标"
for src in "${ENTRIES[@]}"; do
  size=$(du -sh "$src" 2>/dev/null | cut -f1)
  printf '%-20s %14s  %s/%s\n' "$(basename "$src")" "$size" "$DST" "$(basename "$src")"
done

# ---------- 目标冲突预检 ----------
CONFLICTS=""
for src in "${ENTRIES[@]}"; do
  name=$(basename "$src")
  [ -e "$DST/$name" ] && CONFLICTS="$CONFLICTS $name"
done
if [ -n "$CONFLICTS" ]; then
  echo ""
  echo "注意：目标目录已存在同名条目，将跳过而不覆盖：$CONFLICTS" >&2
fi

# ---------- 执行 ----------
if [ "$DRY_RUN" = true ]; then
  echo ""
  echo "这是 DryRun 预览，未执行任何移动。"
  exit 0
fi

read -r -p "确认执行迁移？此操作不可逆（源数据将被移走）[y/N] " ans
case "$ans" in
  y|Y|yes|YES) ;;
  *) echo "已取消。" ; exit 0 ;;
esac

mkdir -p "$DST"
log() { echo "$*" | tee -a "$LOG" ; }
: > "$LOG"
echo "" | tee -a "$LOG"
log "----- 开始迁移（$(date '+%F %T')）-----"

moved=0
skipped=0
for src in "${ENTRIES[@]}"; do
  name=$(basename "$src")
  dst="$DST/$name"
  if [ -e "$dst" ]; then
    log "跳过 $name（目标已存在）"
    skipped=$((skipped+1))
    continue
  fi
  if mv "$src" "$dst" 2>/dev/null; then
    log "已迁移 $name"
    moved=$((moved+1))
  else
    log "迁移失败！$name"
  fi
done

# ---------- 结果摘要 ----------
echo "" | tee -a "$LOG"
log "----- 迁移结果 -----"
log "成功: $moved  跳过(目标已存在): $skipped"
log "日志: $LOG"
if [ "$(ls -A "$SRC" 2>/dev/null | wc -l)" -eq 0 ]; then
  rmdir "$SRC" 2>/dev/null || true
  echo "旧目录 $SRC 已清空并删除。" | tee -a "$LOG"
else
  echo "旧目录 $SRC 仍有剩余条目（被跳过或迁移失败），请人工检查。" >&2
fi