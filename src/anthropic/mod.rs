// Copyright (c) 2026 Harllan He. Licensed under MIT.
//! Anthropic API 兼容服务模块
//!
//! 提供与 Anthropic Claude API 兼容的 HTTP 服务端点。
//!
//! # 支持的端点
//!
//! ## 标准端点 (/v1)
//! - `GET /v1/models` - 获取可用模型列表
//! - `POST /v1/messages` - 创建消息（对话）
//! - `POST /v1/messages/count_tokens` - 计算 token 数量
//!
//! ## Claude Code 兼容端点 (/cc/v1)
//! - `POST /cc/v1/messages` - 创建消息（流式响应实时转发，额外带 300s 上游挂起保护）
//! - `POST /cc/v1/messages/count_tokens` - 计算 token 数量（与 /v1 相同）
//!
//! # 使用示例
//! ```rust,ignore
//! use kiro_rs::anthropic;
//!
//! let app = anthropic::create_router("your-api-key");
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! ```

pub(crate) mod converter;
pub(crate) mod handlers;
pub mod middleware;
mod router;
mod stream;
pub mod types;
mod websearch;

pub use converter::{
    DEFAULT_WRITE_CHUNKING_MAX_LINES, set_write_chunking_max_lines, write_chunking_max_lines,
};
pub use router::create_router_with_provider_and_state;
pub use stream::{
    DEFAULT_CLIENT_CACHE_READ_BASE, DEFAULT_CLIENT_CACHE_READ_MULTIPLIER,
    DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE, client_token_display_config,
    client_token_display_config_with_cache_base, client_token_display_config_with_cache_config,
    set_client_token_display_config, set_client_token_display_config_with_base,
    set_client_token_display_config_with_base_and_multiplier, set_thinking_as_text,
    thinking_as_text_enabled, validate_client_cache_read_base,
    validate_client_cache_read_multiplier, validate_client_token_display_scale,
};
pub(crate) use stream::scale_cache_read_for_client;

#[cfg(test)]
pub(crate) use converter::convert_request;
