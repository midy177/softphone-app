#!/bin/bash
# run-dev.sh - macOS 兼容版本
LOG_DIR="./logs"
mkdir -p "$LOG_DIR"
rm -rf $LOG_DIR/*.log
echo "启动开发服务器，日志输出到: $LOG_DIR"
echo "同时也在控制台显示..."
echo "=========================================="

# macOS 兼容的 script 命令写法
script -q "$LOG_DIR/dev.log" bun run tauri dev
