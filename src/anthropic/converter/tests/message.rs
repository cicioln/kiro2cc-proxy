//! converter 测试：消息内容解析（tool_result 内嵌图片提升到 images）
#![cfg(test)]

use super::super::convert::convert_request;
use super::super::message::extract_tool_result_content;
use crate::anthropic::types::{Message as AnthropicMessage, MessagesRequest};
use crate::kiro::model::requests::conversation::Message;

/// 构造只含给定消息序列的最小请求
fn request(messages: Vec<(&str, serde_json::Value)>) -> MessagesRequest {
    MessagesRequest {
        model: "claude-sonnet-4".to_string(),
        max_tokens: 1024,
        messages: messages
            .into_iter()
            .map(|(role, content)| AnthropicMessage {
                role: role.to_string(),
                content,
            })
            .collect(),
        stream: false,
        system: None,
        tools: None,
        tool_choice: None,
        thinking: None,
        output_config: None,
        metadata: None,
    }
}

/// 读图类工具调用的一轮：assistant 发起 tool_use，user 回传给定内容的 tool_result
fn image_tool_turns(result_content: serde_json::Value) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("user", serde_json::json!("看一下这张图")),
        (
            "assistant",
            serde_json::json!([
                {"type": "tool_use", "id": "toolu_img", "name": "Read", "input": {"file_path": "/tmp/a.png"}}
            ]),
        ),
        (
            "user",
            serde_json::json!([
                {"type": "tool_result", "tool_use_id": "toolu_img", "content": result_content}
            ]),
        ),
    ]
}

fn png_block(data: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "image",
        "source": {"type": "base64", "media_type": "image/png", "data": data}
    })
}

#[test]
fn test_tool_result_image_promoted_to_current_message() {
    // Claude Code 的 Read 读图片时，tool_result.content 只有一个 image 块
    let req = request(image_tool_turns(serde_json::json!([png_block("QUJD")])));
    let state = convert_request(&req).unwrap().conversation_state;
    let msg = &state.current_message.user_input_message;

    assert_eq!(
        msg.images.len(),
        1,
        "tool_result 内的图片应提升到当前消息 images"
    );
    assert_eq!(msg.images[0].format, "png");
    assert_eq!(msg.images[0].source.bytes, "QUJD");

    let results = &msg.user_input_message_context.tool_results;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tool_use_id, "toolu_img");
    assert_eq!(results[0].content[0]["text"], "");
    assert_eq!(msg.content, "(tool result above)");
}

#[test]
fn test_tool_result_text_kept_alongside_image() {
    // 浏览器截图类工具：文本说明与图片混排
    let req = request(image_tool_turns(serde_json::json!([
        {"type": "text", "text": "Took the viewport screenshot"},
        png_block("SU1H"),
        {"type": "text", "text": "[Image: source: /tmp/shot.png]"}
    ])));
    let state = convert_request(&req).unwrap().conversation_state;
    let msg = &state.current_message.user_input_message;

    assert_eq!(msg.images.len(), 1);
    assert_eq!(msg.images[0].source.bytes, "SU1H");
    assert_eq!(
        msg.user_input_message_context.tool_results[0].content[0]["text"],
        "Took the viewport screenshot\n[Image: source: /tmp/shot.png]"
    );
}

#[test]
fn test_history_tool_result_image_kept_in_history() {
    let mut turns = image_tool_turns(serde_json::json!([png_block("QUJD")]));
    turns.push(("assistant", serde_json::json!("图里是一个登录页")));
    turns.push(("user", serde_json::json!("继续")));
    let state = convert_request(&request(turns)).unwrap().conversation_state;

    let history_images: Vec<_> = state
        .history
        .iter()
        .filter_map(|m| match m {
            Message::User(u) => Some(&u.user_input_message.images),
            Message::Assistant(_) => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        history_images.len(),
        1,
        "历史 tool_result 中的图片应保留在对应 user 消息上"
    );
    assert_eq!(history_images[0].source.bytes, "QUJD");
    assert!(state.current_message.user_input_message.images.is_empty());
}

#[test]
fn test_tool_result_unsupported_image_dropped() {
    let (text, images) = extract_tool_result_content(&Some(serde_json::json!([
        {"type": "text", "text": "ok"},
        {"type": "image", "source": {"type": "base64", "media_type": "image/svg+xml", "data": "PHN2Zz4="}},
        {"type": "image", "source": {"type": "url", "url": "https://example.com/a.png"}}
    ])));
    assert_eq!(text, "ok");
    assert!(images.is_empty(), "不支持的格式或非 base64 来源不应上送");
}

#[test]
fn test_text_only_tool_result_unchanged() {
    let (text, images) = extract_tool_result_content(&Some(serde_json::json!("plain output")));
    assert_eq!(text, "plain output");
    assert!(images.is_empty());

    // 无图片请求序列化后不出现 images 字段，与修复前逐字节一致
    let req = request(image_tool_turns(serde_json::json!([
        {"type": "text", "text": "file content"}
    ])));
    let state = convert_request(&req).unwrap().conversation_state;
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("\"images\""));
}
