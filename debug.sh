#!/bin/bash
# run-dev.sh - 跨平台兼容版本
LOG_DIR="./logs"
mkdir -p "$LOG_DIR"
rm -rf $LOG_DIR/*.log
echo "启动开发服务器，日志输出到: $LOG_DIR"
echo "同时也在控制台显示..."
echo "=========================================="

# 检测操作系统
OS="$(uname -s)"
case "$OS" in
    Darwin*)    # macOS
        echo "检测到 macOS 系统"
        script -q "$LOG_DIR/dev.log" bun run tauri dev
        ;;
    Linux*)     # Linux
        echo "检测到 Linux 系统"
        if [[ -n "$WSL_DISTRO_NAME" ]]; then
            echo "检测到 WSL 环境"
            script -f -c "bun run tauri dev" "$LOG_DIR/dev.log"
        else
            script -f -c "bun run tauri dev" "$LOG_DIR/dev.log"
        fi
        ;;
    CYGWIN*|MINGW*|MSYS*)  # Windows (Git Bash, Cygwin, MSYS2)
        echo "检测到 Windows 系统 (Git Bash/Cygwin/MSYS2)"
        # Windows 下使用 tee
        bun run tauri dev 2>&1 | tee "$LOG_DIR/dev.log"
        ;;
    *)
        echo "未知操作系统: $OS"
        echo "尝试使用 tee 命令..."
        # 回退到 tee 方案
        if command -v tee &> /dev/null; then
            bun run tauri dev 2>&1 | tee "$LOG_DIR/dev.log"
        else
            echo "错误: 没有可用的日志记录方法"
            bun run tauri dev
        fi
        ;;
esac