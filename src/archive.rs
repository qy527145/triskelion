//! 压缩体格式探测与压缩参数。client / server 双侧复用。
//!
//! 平台统一采用 **zstd（Zstandard）** 作为技能压缩体与「全量资源包」的压缩算法：
//! 在与 gzip 相近甚至更快的压缩速度下取得显著更高的压缩比，解压速度数倍于 gzip，
//! 是当前工程实践中「高压缩比 + 高性能」的最佳折中。
//!
//! 旧版历史压缩体为 gzip（`.tar.gz`），按 sha256 内容寻址落盘后无法重命名；故解码
//! 路径按「魔数」自动识别 zstd / gzip，保证升级后历史数据仍可正常拉取与迁移。

/// zstd 压缩级别。19 处于「常规档」上限（20~22 为 ultra 档，收益递减且显著变慢），
/// 是高压缩比与可接受耗时之间的甜点；配合多线程编码进一步摊薄大包的压缩耗时。
pub const ZSTD_LEVEL: i32 = 19;

/// 已识别的压缩体格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Zstandard（魔数 0x28 B5 2F FD）。
    Zstd,
    /// gzip（魔数 0x1F 8B）——历史遗留压缩体。
    Gzip,
    /// ZIP（魔数 `PK\x03\x04`）——用户在 Web 端拖入的系统压缩包。
    Zip,
    /// 无法识别。
    Unknown,
}

/// 按文件头魔数识别压缩格式。
pub fn detect(bytes: &[u8]) -> Format {
    if bytes.len() >= 4 && bytes[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        Format::Zstd
    } else if bytes.len() >= 2 && bytes[..2] == [0x1F, 0x8B] {
        Format::Gzip
    } else if bytes.len() >= 4 && bytes[..4] == [0x50, 0x4B, 0x03, 0x04] {
        Format::Zip
    } else {
        Format::Unknown
    }
}

/// 依据内容选择内容寻址落盘时的扩展名（不影响解码，仅便于人工辨识）。
pub fn blob_extension(bytes: &[u8]) -> &'static str {
    match detect(bytes) {
        Format::Zstd => "tar.zst",
        Format::Gzip => "tar.gz",
        // ZIP 仅出现在上传解包途中，落盘前都会重打成 tar.zst；兜底给个可辨识扩展名。
        Format::Zip => "zip",
        Format::Unknown => "bin",
    }
}
