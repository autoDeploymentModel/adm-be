#!/usr/bin/env python3
"""NCCL all-reduce 测试：在两台机器上同时运行，验证跨机 NCCL TCP 通信是否正常。

用法（先停掉 SGLang 容器，否则 6464 被占用）：
  rank 0 (192.168.177.11) 上执行：
    NCCL_DEBUG=TRACE NCCL_MAX_NCHANNELS=8 NCCL_SOCKET_IFNAME=enp1s0f1np1 \
      python3 nccl_test.py 0 [端口]

  rank 1 (192.168.177.12) 上同时执行：
    NCCL_DEBUG=TRACE NCCL_MAX_NCHANNELS=8 NCCL_SOCKET_IFNAME=enp1s0f1np1 \
      python3 nccl_test.py 1 [端口]

  - 端口默认 6464；与占用冲突时传同一个新端口（两端一致）
  - NCCL 日志出现 "Init COMPLETE" 且末尾 "SUCCESS: tensor([3.], device='cuda:0')"
    => 网络栈正常，问题在 SGLang 层（对比启动参数/镜像）
  - 与 SGLang 一样卡在 Channel 打点后、无 "Init COMPLETE"、无流量
    => NCCL/驱动/容器栈问题，直接试 RoCE 形态（--device /dev/infiniband + memlock + IPC_LOCK）
"""
import sys
import torch
import torch.distributed as dist

RANK = int(sys.argv[1]) if len(sys.argv) > 1 else 0
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 6464
WORLD_SIZE = 2
INIT_METHOD = f"tcp://192.168.177.11:{PORT}"

print(f"[Rank {RANK}] 初始化 NCCL...", flush=True)
dist.init_process_group(
    backend="nccl",
    init_method=INIT_METHOD,
    rank=RANK,
    world_size=WORLD_SIZE,
)

print(f"[Rank {RANK}] NCCL 初始化完成，开始 all-reduce...", flush=True)
t = torch.tensor([float(RANK + 1)]).cuda()
dist.all_reduce(t)
print(f"[Rank {RANK}] SUCCESS: {t}", flush=True)

dist.destroy_process_group()
print(f"[Rank {RANK}] 完成", flush=True)
