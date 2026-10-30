//! 统一错误类型。

/// 核心层错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("解析失败：{0}")]
    Parse(String),
    #[error("校验失败：{0}")]
    Invalid(String),
    #[error("编码失败：{0}")]
    Encode(String),
    #[error("解码失败：{0}")]
    Decode(String),
    #[error("未知客户端：{0}")]
    UnknownClient(String),
    #[error("该客户端不支持此操作：{0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;
