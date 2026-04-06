//! LLM conversation message types.

use serde::{Deserialize, Serialize};

use crate::tool::ToolExecutionStatus;

/// A message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the sender.
    pub role: Role,
    /// The content of the message.
    pub content: MessageContent,
    /// Whether this message is pinned (protected from overflow trimming).
    ///
    /// Pinned messages are preserved during context overflow recovery,
    /// ensuring critical early messages (system constraints, user rules,
    /// important context) are not lost when trimming.
    #[serde(default)]
    pub pinned: bool,
}

/// The role of a message sender in an LLM conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System prompt.
    System,
    /// Human user.
    User,
    /// AI assistant.
    Assistant,
}

/// Content of a message — can be simple text or structured blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content.
    Text(String),
    /// Structured content blocks.
    Blocks(Vec<ContentBlock>),
}

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// A text block.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
        /// Provider-specific metadata (e.g. Gemini `thoughtSignature`).
        /// Opaque to the core — drivers read/write this to round-trip
        /// fields the provider requires on subsequent requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
    /// An inline base64-encoded image.
    #[serde(rename = "image")]
    Image {
        /// MIME type (e.g. "image/png", "image/jpeg").
        media_type: String,
        /// Base64-encoded image data.
        data: String,
    },
    /// A tool use request from the assistant.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique ID for this tool use.
        id: String,
        /// The tool name.
        name: String,
        /// The tool input parameters.
        input: serde_json::Value,
        /// Provider-specific metadata (e.g. Gemini `thoughtSignature`).
        /// Opaque to the core — drivers read/write this to round-trip
        /// fields the provider requires on subsequent requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
    /// A tool result from executing a tool.
    #[serde(rename = "tool_result")]
    ToolResult {
        /// The tool_use ID this result corresponds to.
        tool_use_id: String,
        /// The tool name (for Gemini FunctionResponse). Empty for legacy sessions.
        #[serde(default)]
        tool_name: String,
        /// The result content.
        content: String,
        /// Whether the tool execution errored.
        is_error: bool,
        /// Detailed execution status.
        #[serde(default)]
        status: ToolExecutionStatus,
        /// Approval request ID, set when status is WaitingApproval.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_request_id: Option<String>,
    },
    /// Extended thinking content block (model's reasoning trace).
    #[serde(rename = "thinking")]
    Thinking {
        /// The thinking/reasoning text.
        thinking: String,
        /// Provider-specific metadata (e.g. Gemini `thoughtSignature`).
        /// Opaque to the core — drivers read/write this to round-trip
        /// fields the provider requires on subsequent requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
    /// Catch-all for unrecognized content block types (forward compatibility).
    #[serde(other)]
    Unknown,
}

/// Allowed image media types.
const ALLOWED_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Maximum decoded image size (5 MB).
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Validate an image content block.
///
/// Checks that the media type is an allowed image format and the
/// base64 data doesn't exceed 5 MB when decoded (~7 MB base64).
pub fn validate_image(media_type: &str, data: &str) -> Result<(), String> {
    if !ALLOWED_IMAGE_TYPES.contains(&media_type) {
        return Err(format!(
            "Unsupported image type '{}'. Allowed: {}",
            media_type,
            ALLOWED_IMAGE_TYPES.join(", ")
        ));
    }
    // Base64 encodes 3 bytes into 4 chars, so max base64 len ≈ MAX_IMAGE_BYTES * 4/3
    let max_b64_len = MAX_IMAGE_BYTES * 4 / 3 + 4; // small padding allowance
    if data.len() > max_b64_len {
        return Err(format!(
            "Image too large: {} bytes base64 (max ~{} bytes for {} MB decoded)",
            data.len(),
            max_b64_len,
            MAX_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

impl MessageContent {
    /// Create simple text content.
    pub fn text(content: impl Into<String>) -> Self {
        MessageContent::Text(content.into())
    }

    /// Get the total character length of text in this content.
    pub fn text_length(&self) -> usize {
        match self {
            MessageContent::Text(s) => s.len(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text, .. } => text.len(),
                    ContentBlock::ToolResult { content, .. } => content.len(),
                    ContentBlock::Thinking { thinking, .. } => thinking.len(),
                    ContentBlock::ToolUse { .. }
                    | ContentBlock::Image { .. }
                    | ContentBlock::Unknown => 0,
                })
                .sum(),
        }
    }

    /// Extract all text content as a single string.
    pub fn text_content(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Check whether this content contains any image blocks.
    pub fn has_images(&self) -> bool {
        match self {
            MessageContent::Text(_) => false,
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. })),
        }
    }

    /// Replace all image blocks with lightweight text placeholders.
    ///
    /// After an image has been sent to the LLM, the base64 data (~56K tokens
    /// per image) is no longer needed in session history.  This replaces each
    /// `ContentBlock::Image` with a small text note so the conversation context
    /// is preserved without the massive token cost.
    ///
    /// Returns `true` if any images were replaced.
    pub fn strip_images(&mut self) -> bool {
        match self {
            MessageContent::Text(_) => false,
            MessageContent::Blocks(blocks) => {
                let mut stripped = false;
                for block in blocks.iter_mut() {
                    if let ContentBlock::Image { media_type, .. } = block {
                        let placeholder = format!("[Image ({media_type}) previously processed]");
                        *block = ContentBlock::Text {
                            text: placeholder,
                            provider_metadata: None,
                        };
                        stripped = true;
                    }
                }
                stripped
            }
        }
    }
}

impl Message {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: MessageContent::Text(content.into()),
            pinned: false,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Text(content.into()),
            pinned: false,
        }
    }

    /// Create a user message with structured content blocks (e.g. text + images).
    pub fn user_with_blocks(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Blocks(blocks),
            pinned: false,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(content.into()),
            pinned: false,
        }
    }
}

/// Why the LLM stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its turn.
    EndTurn,
    /// The model wants to use a tool.
    ToolUse,
    /// The model hit the token limit.
    MaxTokens,
    /// The model hit a stop sequence.
    StopSequence,
}

/// Token usage information from an LLM call.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Tokens used for the input/prompt.
    pub input_tokens: u64,
    /// Tokens generated in the output.
    pub output_tokens: u64,
    /// Tokens written to the prompt cache (Anthropic `cache_creation_input_tokens`).
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    /// Tokens read from the prompt cache (Anthropic `cache_read_input_tokens`,
    /// OpenAI `prompt_tokens_details.cached_tokens`).
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl TokenUsage {
    /// Total tokens used.
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Reply directives extracted from agent output.
///
/// These control how the response is delivered back to the user/channel:
/// - `reply_to`: reply to a specific message ID
/// - `current_thread`: reply in the current thread
/// - `silent`: suppress the response entirely
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyDirectives {
    /// Reply to a specific message ID.
    pub reply_to: Option<String>,
    /// Reply in the current thread.
    pub current_thread: bool,
    /// Suppress the response from being sent.
    pub silent: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        match msg.content {
            MessageContent::Text(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_token_usage() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        assert_eq!(usage.total(), 150);
    }

    #[test]
    fn test_token_usage_default_cache_fields() {
        let usage = TokenUsage::default();
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_token_usage_cache_deserialization() {
        let json = r#"{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":30,"cache_read_input_tokens":70}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cache_creation_input_tokens, 30);
        assert_eq!(usage.cache_read_input_tokens, 70);
    }

    #[test]
    fn test_token_usage_cache_deserialization_missing_fields() {
        let json = r#"{"input_tokens":100,"output_tokens":50}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_validate_image_valid() {
        assert!(validate_image("image/png", "iVBORw0KGgo=").is_ok());
        assert!(validate_image("image/jpeg", "data").is_ok());
        assert!(validate_image("image/gif", "data").is_ok());
        assert!(validate_image("image/webp", "data").is_ok());
    }

    #[test]
    fn test_validate_image_bad_type() {
        let err = validate_image("image/svg+xml", "data").unwrap_err();
        assert!(err.contains("Unsupported image type"));
        let err = validate_image("text/plain", "data").unwrap_err();
        assert!(err.contains("Unsupported image type"));
    }

    #[test]
    fn test_validate_image_too_large() {
        let huge = "A".repeat(8_000_000); // ~6MB base64
        let err = validate_image("image/png", &huge).unwrap_err();
        assert!(err.contains("too large"));
    }

    #[test]
    fn test_content_block_image_serde() {
        let block = ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["media_type"], "image/png");
    }

    #[test]
    fn test_content_block_unknown_deser() {
        let json = serde_json::json!({"type": "future_block_type"});
        let block: ContentBlock = serde_json::from_value(json).unwrap();
        assert!(matches!(block, ContentBlock::Unknown));
    }

    #[test]
    fn test_user_with_blocks() {
        let blocks = vec![
            ContentBlock::Text {
                text: "What is in this image?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "base64data".to_string(),
            },
        ];
        let msg = Message::user_with_blocks(blocks);
        assert_eq!(msg.role, Role::User);
        match msg.content {
            MessageContent::Blocks(ref b) => {
                assert_eq!(b.len(), 2);
                assert!(
                    matches!(&b[0], ContentBlock::Text { text, .. } if text == "What is in this image?")
                );
                assert!(
                    matches!(&b[1], ContentBlock::Image { media_type, .. } if media_type == "image/jpeg")
                );
            }
            _ => panic!("Expected blocks content"),
        }
    }

    #[test]
    fn test_has_images_text_content() {
        let content = MessageContent::text("Hello");
        assert!(!content.has_images());
    }

    #[test]
    fn test_has_images_blocks_without_image() {
        let content = MessageContent::Blocks(vec![ContentBlock::Text {
            text: "Hello".to_string(),
            provider_metadata: None,
        }]);
        assert!(!content.has_images());
    }

    #[test]
    fn test_has_images_blocks_with_image() {
        let content = MessageContent::Blocks(vec![
            ContentBlock::Text {
                text: "What is this?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "base64data".to_string(),
            },
        ]);
        assert!(content.has_images());
    }

    #[test]
    fn test_strip_images_text_content() {
        let mut content = MessageContent::text("Hello");
        assert!(!content.strip_images());
        assert_eq!(content.text_content(), "Hello");
    }

    #[test]
    fn test_strip_images_no_images() {
        let mut content = MessageContent::Blocks(vec![ContentBlock::Text {
            text: "Hello".to_string(),
            provider_metadata: None,
        }]);
        assert!(!content.strip_images());
    }

    #[test]
    fn test_strip_images_replaces_image_with_placeholder() {
        let mut content = MessageContent::Blocks(vec![
            ContentBlock::Text {
                text: "What is this?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "huge_base64_data_here".to_string(),
            },
        ]);
        assert!(content.strip_images());
        // Image block should now be a text placeholder
        assert!(!content.has_images());
        let text = content.text_content();
        assert!(text.contains("[Image (image/jpeg) previously processed]"));
        // Original text should still be present
        assert!(text.contains("What is this?"));
    }

    #[test]
    fn test_strip_images_multiple_images() {
        let mut content = MessageContent::Blocks(vec![
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "data1".to_string(),
            },
            ContentBlock::Text {
                text: "between".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "data2".to_string(),
            },
        ]);
        assert!(content.strip_images());
        assert!(!content.has_images());
        let text = content.text_content();
        assert!(text.contains("[Image (image/png) previously processed]"));
        assert!(text.contains("[Image (image/jpeg) previously processed]"));
        assert!(text.contains("between"));
    }

    // -----------------------------------------------------------------------
    // ContentBlock::ToolResult tests (Step 1 from plan)
    // -----------------------------------------------------------------------

    #[test]
    fn test_content_block_tool_result_serde() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "toolu_abc".to_string(),
            tool_name: "shell_exec".to_string(),
            content: "Command output".to_string(),
            is_error: false,
            status: crate::tool::ToolExecutionStatus::Completed,
            approval_request_id: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        match deserialized {
            ContentBlock::ToolResult {
                tool_use_id,
                status,
                approval_request_id,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_abc");
                assert_eq!(status, crate::tool::ToolExecutionStatus::Completed);
                assert!(approval_request_id.is_none());
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    #[test]
    fn test_content_block_tool_result_deserialization_old_format() {
        // Old format without status and approval_request_id fields
        let json = r#"{
            "type": "tool_result",
            "tool_use_id": "toolu_old",
            "tool_name": "file_read",
            "content": "File contents",
            "is_error": false
        }"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                status,
                approval_request_id,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_old");
                assert_eq!(status, crate::tool::ToolExecutionStatus::Completed); // Default
                assert!(approval_request_id.is_none()); // Default
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    #[test]
    fn test_content_block_tool_result_with_approval_request_id() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "toolu_abc".to_string(),
            tool_name: "shell_exec".to_string(),
            content: "Waiting for approval".to_string(),
            is_error: false,
            status: crate::tool::ToolExecutionStatus::WaitingApproval,
            approval_request_id: Some("req-123".to_string()),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("waiting_approval"));
        assert!(json.contains("req-123"));

        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        match deserialized {
            ContentBlock::ToolResult {
                approval_request_id,
                ..
            } => {
                assert_eq!(approval_request_id, Some("req-123".to_string()));
            }
            _ => panic!("Expected ToolResult block"),
        }
    }
}
