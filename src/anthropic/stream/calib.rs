//! Token 显示校准：按模型确定 context window 与空响应阈值，支持客户端直通和可配置的输入/缓存 token 展示缩放。

/// 所有模型统一按 100 万 token 上下文窗口计算。
///
/// 历史上按模型分支返回 200K/1M；本变更改为统一 1M，与"contextUsage 本地化"决策一致：
/// final_input_tokens 不再依赖 Kiro `contextUsageEvent` 反算；如需差异化窗口
/// 可恢复 match 分支。
pub(crate) fn context_window_for_model(_model: &str) -> i32 {
    1_000_000
}

/// 空响应判定为「上下文过大」的输入 token 阈值（取窗口的 28%）。
///
/// 实测 input≈297K 时上游已频繁返回空/极短响应（4~13 tokens 无工具调用），
/// 取窗口的 28%（≈280K for 1M 窗口）作为判定阈值。
pub(crate) fn empty_response_oversized_threshold(model: &str) -> i32 {
    (context_window_for_model(model) as f64 * 0.28) as i32
}

/// "近似空响应"的 output token 阈值。
///
/// 当上下文压力大时，模型可能返回极短的无意义文本（如 4~13 tokens）而非工具调用，
/// 导致客户端 agentic 循环卡住。output < 此阈值且无工具调用时，视为近似空响应。
pub(crate) const NEAR_EMPTY_OUTPUT_THRESHOLD: i32 = 30;

/// 返回给客户端的输入/缓存 token 默认展示系数。
///
/// 仅影响客户端可见的 usage.input_tokens / cache_* 字段，内部计费与
/// usage_tracker 入库保持原始口径。所有模型使用同一系数；output_tokens 不缩放。
/// 配置 `clientTokenDisplayScale` 或管理设置页可覆盖此默认值。
///
/// 历史实测 Claude Code 按 200K 窗口计算 Ctx%，在约 82%（展示 164,000）时
/// auto-compact；默认系数对应约 246,357 个未缩放输入 token。
/// 修改系数或开启直通会改变返回的 usage，依赖这些值的客户端可能提前或延后压缩；
/// 这不是服务端上下文窗口或自动压缩阈值设置。
pub const DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE: f64 = 0.6657;

/// 客户端可见的缓存读取模拟基数，单位为 token。
///
/// 该值只追加到返回给客户端的 `cache_read_input_tokens`，不参与上游请求、内部
/// 用量记录或计费。默认关闭（0），可通过配置文件或 Admin API 自定义。
pub const DEFAULT_CLIENT_CACHE_READ_BASE: i32 = 0;

/// 客户端可见缓存读取量在现有展示口径基础上使用的倍率。
pub const DEFAULT_CLIENT_CACHE_READ_MULTIPLIER: f64 = 1.0;

/// 客户端展示配置（直通开关、关闭直通时使用的系数、模拟缓存读取基数与倍率）。
///
/// 用同一把锁读写四者，避免热更新时读取到一半旧、一半新的配置。
/// main 启动时初始化，Admin API 运行时更新；直通开启不会丢弃已保存的系数。
static CLIENT_TOKEN_DISPLAY_CONFIG: parking_lot::RwLock<(bool, f64, i32, f64)> =
    parking_lot::RwLock::new((
        false,
        DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE,
        DEFAULT_CLIENT_CACHE_READ_BASE,
        DEFAULT_CLIENT_CACHE_READ_MULTIPLIER,
    ));

/// 所有配置入口共用的展示系数校验。
pub fn validate_client_token_display_scale(scale: f64) -> Result<(), &'static str> {
    if scale.is_finite() && scale > 0.0 && scale <= 1.0 {
        Ok(())
    } else {
        Err("clientTokenDisplayScale 必须是大于 0 且不超过 1 的有限数")
    }
}

/// 设置客户端 token 直通和展示系数（启动初始化、Admin API 热更新）。
pub fn set_client_token_display_config(enabled: bool, display_scale: f64) -> Result<(), &'static str> {
    let (_, _, cache_read_base, cache_read_multiplier) =
        client_token_display_config_with_cache_config();
    set_client_token_display_config_with_base_and_multiplier(
        enabled,
        display_scale,
        cache_read_base,
        cache_read_multiplier,
    )
}

/// 设置客户端 token 展示配置及模拟缓存基数。
pub fn set_client_token_display_config_with_base(
    enabled: bool,
    display_scale: f64,
    cache_read_base: i32,
) -> Result<(), &'static str> {
    let cache_read_multiplier = client_token_display_config_with_cache_config().3;
    set_client_token_display_config_with_base_and_multiplier(
        enabled,
        display_scale,
        cache_read_base,
        cache_read_multiplier,
    )
}

/// 设置客户端 token 展示配置、模拟缓存基数及真实缓存倍率。
pub fn set_client_token_display_config_with_base_and_multiplier(
    enabled: bool,
    display_scale: f64,
    cache_read_base: i32,
    cache_read_multiplier: f64,
) -> Result<(), &'static str> {
    validate_client_token_display_scale(display_scale)?;
    validate_client_cache_read_base(cache_read_base)?;
    validate_client_cache_read_multiplier(cache_read_multiplier)?;
    *CLIENT_TOKEN_DISPLAY_CONFIG.write() = (
        enabled,
        display_scale,
        cache_read_base,
        cache_read_multiplier,
    );
    Ok(())
}

/// 读取当前客户端展示配置的同一份快照。
pub fn client_token_display_config() -> (bool, f64) {
    let (enabled, display_scale, _, _) = *CLIENT_TOKEN_DISPLAY_CONFIG.read();
    (enabled, display_scale)
}

/// 读取客户端 token 展示配置及模拟缓存基数。
pub fn client_token_display_config_with_cache_base() -> (bool, f64, i32) {
    let (enabled, display_scale, cache_read_base, _) =
        client_token_display_config_with_cache_config();
    (enabled, display_scale, cache_read_base)
}

/// 读取客户端 token 展示、模拟缓存基数及真实缓存倍率配置。
pub fn client_token_display_config_with_cache_config() -> (bool, f64, i32, f64) {
    *CLIENT_TOKEN_DISPLAY_CONFIG.read()
}

/// 校验客户端可见的模拟缓存基数。
pub fn validate_client_cache_read_base(base: i32) -> Result<(), &'static str> {
    if base >= 0 {
        Ok(())
    } else {
        Err("cacheReadBase 必须是大于等于 0 的整数")
    }
}

/// 校验客户端可见的真实缓存读取倍率。
pub fn validate_client_cache_read_multiplier(multiplier: f64) -> Result<(), &'static str> {
    if multiplier.is_finite() && multiplier >= 0.0 {
        Ok(())
    } else {
        Err("cacheReadMultiplier 必须是大于等于 0 的有限数")
    }
}

/// Claude Code 计算 Ctx% 时假设的上下文窗口（分母）。
///
/// 属于客户端展示口径，与上游真实窗口（1M）分开。
/// 超窗错误文案里的 maximum 使用该值，并对 N 做 M+1 兜底，避免文案自相矛盾。
pub(crate) const CLIENT_ASSUMED_CONTEXT_WINDOW: i32 = 200_000;

/// 对客户端展示用的 token 值缩放（向上取整保证正数不归零）。
pub(crate) fn scale_for_client(n: i32, _model: &str) -> i32 {
    let (passthrough, display_scale) = client_token_display_config();
    scale_for_client_with_scale(n, passthrough, display_scale)
}

/// 将真实缓存读取量转换为客户端可见值，应用缓存倍率并追加模拟缓存基数。
///
/// 基数位于展示口径（缩放之后），这样设置 `45000` 时客户端始终能看到
/// `45000 + 真实缓存读取量`，不会被默认的展示缩放再次折算。倍率作用于现有展示口径
/// 处理后的真实缓存值。该函数只用于响应
/// 序列化，内部用量记录仍使用未经追加的真实值。
pub(crate) fn scale_cache_read_for_client(n: i32, _model: &str) -> i32 {
    let (passthrough, display_scale, base, multiplier) =
        client_token_display_config_with_cache_config();
    let scaled = scale_for_client_with_scale(n, passthrough, display_scale);
    let scaled = apply_cache_read_multiplier(scaled, multiplier);
    scaled.saturating_add(base)
}

fn apply_cache_read_multiplier(n: i32, multiplier: f64) -> i32 {
    if n <= 0 || multiplier == 1.0 {
        return n.max(0);
    }
    ((n as f64) * multiplier)
        .ceil()
        .max(0.0)
        .min(i32::MAX as f64) as i32
}

fn scale_for_client_with_scale(n: i32, passthrough: bool, display_scale: f64) -> i32 {
    if passthrough || n <= 0 {
        return n.max(0);
    }
    ((n as f64) * display_scale).ceil() as i32
}

/// 现有回归测试使用默认系数，避免修改全局配置干扰并行测试。
#[cfg(test)]
pub(crate) fn scale_for_client_with(n: i32, passthrough: bool) -> i32 {
    scale_for_client_with_scale(n, passthrough, DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE)
}

#[cfg(test)]
pub(crate) fn scale_cache_read_for_client_with(
    n: i32,
    passthrough: bool,
    cache_read_base: i32,
) -> i32 {
    scale_for_client_with_scale(n, passthrough, DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE)
        .saturating_add(cache_read_base)
}

#[cfg(test)]
pub(crate) fn scale_cache_read_for_client_with_multiplier(
    n: i32,
    passthrough: bool,
    cache_read_base: i32,
    cache_read_multiplier: f64,
) -> i32 {
    let scaled =
        scale_for_client_with_scale(n, passthrough, DEFAULT_CLIENT_TOKEN_DISPLAY_SCALE);
    apply_cache_read_multiplier(scaled, cache_read_multiplier).saturating_add(cache_read_base)
}

pub(crate) fn cap_input_tokens(
    context_input_tokens: i32,
    _local_estimate: i32,
    model: &str,
) -> i32 {
    let cap = context_window_for_model(model);
    context_input_tokens.clamp(1, cap)
}

pub fn cap_input_tokens_pub(context_input_tokens: i32, local_estimate: i32, model: &str) -> i32 {
    cap_input_tokens(context_input_tokens, local_estimate, model)
}
