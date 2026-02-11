# 支持的音频编解码器

本应用通过 `audio-codec` crate 和 `rustrtc` 库支持以下 VoIP 音频编解码器：

## 编解码器列表

| 编解码器 | RTP PT | 采样率 | 比特率 | 说明 |
|---------|--------|--------|--------|------|
| **Opus** | 111 (动态) | 48 kHz | 6-510 kbps | 现代高质量编解码器，推荐用于高带宽场景 |
| **G.722** | 9 | 16 kHz | 64 kbps | 宽带编解码器，提供更好的音质 |
| **PCMU (G.711 μ-law)** | 0 | 8 kHz | 64 kbps | 标准北美电话编解码器 |
| **PCMA (G.711 A-law)** | 8 | 8 kHz | 64 kbps | 标准国际电话编解码器 |
| **G.729** | 18 | 8 kHz | 8 kbps | 低带宽编解码器 |
| **Telephone Event** | 101 | 8 kHz | - | DTMF 音调事件 (RFC 4733) |

## 协商优先级

在 SDP 协商中，编解码器按以下顺序优先：

1. Opus (最高音质)
2. G.722 (宽带音质)
3. PCMU (标准音质)
4. PCMA (标准音质)
5. G.729 (低带宽)
6. Telephone Event (辅助功能)

## 配置位置

编解码器在以下文件中配置：

- **WebRTC 能力声明**: `src-tauri/src/webrtc/mod.rs`
  ```rust
  audio: vec![
      AudioCapability::opus(),
      AudioCapability::g722(),
      AudioCapability::pcmu(),
      AudioCapability::pcma(),
      AudioCapability::g729(),
      AudioCapability::telephone_event(),
  ]
  ```

- **编解码器实现**: `src-tauri/src/webrtc/codec.rs`
  - 使用 `audio-codec` crate 提供的统一接口
  - 支持编码/解码、RTP PT 映射、采样率查询等

## 依赖要求

### Opus 支持
Opus 编解码器需要系统安装 **CMake**：

```bash
# macOS
brew install cmake

# 然后重新编译
cargo build
```

### 其他编解码器
- **PCMU/PCMA**: 纯 Rust 实现，无需额外依赖
- **G.722**: 纯 Rust 实现，无需额外依赖
- **G.729**: 使用 C 库包装器，自动编译

## 使用示例

### 编码 PCM 音频
```rust
use audio_codec::CodecType;
use crate::webrtc::codec::CodecTypeExt;

let pcm_samples: Vec<i16> = vec![0; 160]; // 20ms @ 8kHz
let encoded = CodecType::PCMU.encode(&pcm_samples);
```

### 解码音频数据
```rust
let encoded_data: Vec<u8> = vec![/* ... */];
let pcm_samples = CodecType::PCMU.decode(&encoded_data);
```

### 从 RTP Payload Type 识别编解码器
```rust
use crate::webrtc::codec::CodecTypeExt;

if let Some(codec) = <CodecType as CodecTypeExt>::from_payload_type(9) {
    println!("Codec: {:?}", codec); // G722
    println!("Sample rate: {}", codec.default_clock_rate()); // 16000
}
```

## SDP 协商

应用会自动从 SDP 协商中解析编解码器参数：

```rust
let sdp = "v=0\r\nm=audio 5004 RTP/AVP 9\r\na=rtpmap:9 G722/16000\r\na=ptime:20\r\n";
let negotiated = parse_negotiated_codec(sdp);
// negotiated.codec = CodecType::G722
// negotiated.clock_rate = 16000
// negotiated.ptime_ms = 20
```

## 性能数据

在 Apple M2 Pro 上处理 20ms 音频帧的基准测试结果：

- **PCMU/PCMA**: ~50 纳秒
- **G.722**: ~6.1 微秒  
- **Opus**: ~84.1 微秒

## 兼容性

- ✅ 与标准 SIP 服务器兼容 (Asterisk, FreeSWITCH, 等)
- ✅ 支持动态 Payload Type 协商
- ✅ 自动处理采样率转换
- ✅ 支持可变帧长度 (ptime)

## 相关文档

- [audio-codec crate](https://crates.io/crates/audio-codec)
- [rustrtc](https://github.com/restsend/rustrtc)
- [RFC 3551 - RTP Profile for Audio and Video Conferences](https://www.rfc-editor.org/rfc/rfc3551)
- [RFC 7587 - RTP Payload Format for Opus](https://www.rfc-editor.org/rfc/rfc7587)
