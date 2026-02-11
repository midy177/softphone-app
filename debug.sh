#!/bin/bash
# run-dev.sh
LOG_DIR="./logs"
mkdir -p "$LOG_DIR"
rm -rf $LOG_DIR/*.log
echo "启动开发服务器，日志输出到: $LOG_DIR"
echo "同时也在控制台显示..."
echo "=========================================="

# 执行并tee到文件
script -f logs/dev.log -c "bun run tauri dev"