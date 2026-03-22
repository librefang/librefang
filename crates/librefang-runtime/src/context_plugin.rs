//! Pluggable context plugin trait for modular context management.
//!
//! [`ContextPlugin`] provides fine-grained hooks into the context lifecycle,
//! complementing the higher-level [`ContextEngine`](crate::context_engine::ContextEngine).
//! While the context engine owns the full assembly/compaction pipeline,
//! context plugins are lightweight interceptors that can be stacked:
//!
//! - **`on_pre_completion`** — modify/filter messages before each LLM call
//! - **`on_overflow`** — handle context overflow (return `true` if handled)
//! - **`on_post_turn`** — observe or persist state after each turn
//!
//! # Default Plugin
//!
//! [`DefaultContextPlugin`] wraps the existing 4-stage overflow recovery
//! pipeline from [`context_overflow`](crate::context_overflow), ensuring
//! backward compatibility when no custom plugins are configured.
//!
//! # Plugin Registry
//!
//! [`ContextPluginRegistry`] holds an ordered list of plugins and dispatches
//! lifecycle calls in order. For overflow, it short-circuits on the first
//! plugin that reports successful handling.

use async_trait::async_trait;
use librefang_types::message::Message;
use librefang_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::context_overflow::{recover_from_overflow, RecoveryStage};

/// A pluggable hook into the context management lifecycle.
///
/// Plugins are called by the agent loop at well-defined points. Multiple
/// plugins can be composed via [`ContextPluginRegistry`].
#[async_trait]
pub trait ContextPlugin: Send + Sync {
    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Called before messages are sent to the LLM.
    ///
    /// Plugins can modify, reorder, or filter messages in place.
    /// `context_window` is the model's context window size in tokens.
    async fn on_pre_completion(
        &self,
        messages: &mut Vec<Message>,
        context_window: usize,
    ) -> Result<(), String>;

    /// Called when context overflow is detected.
    ///
    /// Plugins should attempt to reduce context size. Return `Ok(true)`
    /// if the overflow was handled (no further plugins will be called),
    /// or `Ok(false)` to let the next plugin try.
    async fn on_overflow(
        &self,
        messages: &mut Vec<Message>,
        context_window: usize,
    ) -> Result<bool, String>;

    /// Called after each turn completes.
    ///
    /// Use this to persist summaries, update indexes, or perform
    /// background bookkeeping. Default implementation is a no-op.
    async fn on_post_turn(
        &self,
        messages: &[Message],
        response: &str,
    ) -> Result<(), String> {
        let _ = (messages, response);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DefaultContextPlugin — wraps existing 4-stage overflow recovery
// ---------------------------------------------------------------------------

/// Default plugin that wraps the existing 4-stage overflow recovery pipeline.
///
/// Stages:
/// 1. Auto-compact via message trimming (keep recent, drop old)
/// 2. Aggressive overflow compaction (drop all but last N)
/// 3. Truncate historical tool results to 2K chars each
/// 4. Return error suggesting /reset or /compact
pub struct DefaultContextPlugin {
    /// System prompt used for token estimation during overflow recovery.
    system_prompt: String,
    /// Tool definitions used for token estimation during overflow recovery.
    tools: Vec<ToolDefinition>,
}

impl DefaultContextPlugin {
    /// Create a new default plugin with the given system prompt and tools.
    pub fn new(system_prompt: String, tools: Vec<ToolDefinition>) -> Self {
        Self {
            system_prompt,
            tools,
        }
    }
}

#[async_trait]
impl ContextPlugin for DefaultContextPlugin {
    fn name(&self) -> &str {
        "default-overflow-recovery"
    }

    async fn on_pre_completion(
        &self,
        _messages: &mut Vec<Message>,
        _context_window: usize,
    ) -> Result<(), String> {
        // Default plugin does not modify messages pre-completion;
        // overflow handling is done in on_overflow.
        Ok(())
    }

    async fn on_overflow(
        &self,
        messages: &mut Vec<Message>,
        context_window: usize,
    ) -> Result<bool, String> {
        let stage = recover_from_overflow(
            messages,
            &self.system_prompt,
            &self.tools,
            context_window,
        );

        match &stage {
            RecoveryStage::None => Ok(false),
            RecoveryStage::FinalError => {
                warn!("DefaultContextPlugin: all 4 recovery stages exhausted");
                // We attempted recovery but couldn't resolve it — report as handled
                // so the caller can decide how to surface the error.
                Ok(true)
            }
            RecoveryStage::AutoCompaction { removed } => {
                debug!(removed, "DefaultContextPlugin: stage 1 auto-compaction");
                Ok(true)
            }
            RecoveryStage::OverflowCompaction { removed } => {
                debug!(removed, "DefaultContextPlugin: stage 2 aggressive compaction");
                Ok(true)
            }
            RecoveryStage::ToolResultTruncation { truncated } => {
                debug!(truncated, "DefaultContextPlugin: stage 3 tool truncation");
                Ok(true)
            }
        }
    }

    async fn on_post_turn(
        &self,
        _messages: &[Message],
        _response: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ContextPluginRegistry — ordered dispatch of plugin hooks
// ---------------------------------------------------------------------------

/// An ordered collection of context plugins.
///
/// Dispatches lifecycle calls to each plugin in registration order.
/// For overflow handling, short-circuits on the first plugin that
/// reports successful handling (`Ok(true)`).
pub struct ContextPluginRegistry {
    plugins: Vec<Arc<dyn ContextPlugin>>,
}

impl ContextPluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Add a plugin to the end of the chain.
    pub fn register(&mut self, plugin: Arc<dyn ContextPlugin>) {
        debug!(plugin = plugin.name(), "Registering context plugin");
        self.plugins.push(plugin);
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry has no plugins.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Call `on_pre_completion` on every plugin in order.
    ///
    /// If any plugin returns an error, the chain stops and the error
    /// is propagated.
    pub async fn run_pre_completion(
        &self,
        messages: &mut Vec<Message>,
        context_window: usize,
    ) -> Result<(), String> {
        for plugin in &self.plugins {
            plugin.on_pre_completion(messages, context_window).await?;
        }
        Ok(())
    }

    /// Call `on_overflow` on plugins in order until one handles it.
    ///
    /// Returns `true` if any plugin handled the overflow, `false` if
    /// no plugin could resolve it.
    pub async fn run_overflow(
        &self,
        messages: &mut Vec<Message>,
        context_window: usize,
    ) -> Result<bool, String> {
        for plugin in &self.plugins {
            let handled = plugin.on_overflow(messages, context_window).await?;
            if handled {
                debug!(
                    plugin = plugin.name(),
                    "Context overflow handled by plugin"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Call `on_post_turn` on every plugin in order.
    ///
    /// Errors are logged but do not stop the chain — post-turn hooks
    /// are best-effort.
    pub async fn run_post_turn(
        &self,
        messages: &[Message],
        response: &str,
    ) {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_post_turn(messages, response).await {
                warn!(
                    plugin = plugin.name(),
                    error = %e,
                    "Context plugin on_post_turn failed (continuing)"
                );
            }
        }
    }
}

impl Default for ContextPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{Message, MessageContent, Role};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_messages(count: usize, size_each: usize) -> Vec<Message> {
        (0..count)
            .map(|i| {
                let text = format!("msg{}: {}", i, "x".repeat(size_each));
                Message {
                    role: if i % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    content: MessageContent::Text(text),
                    pinned: false,
                }
            })
            .collect()
    }

    /// A test plugin that tracks whether each hook was called.
    struct TrackingPlugin {
        name: &'static str,
        pre_completion_called: AtomicBool,
        overflow_called: AtomicBool,
        post_turn_called: AtomicBool,
        handle_overflow: bool,
    }

    impl TrackingPlugin {
        fn new(name: &'static str, handle_overflow: bool) -> Self {
            Self {
                name,
                pre_completion_called: AtomicBool::new(false),
                overflow_called: AtomicBool::new(false),
                post_turn_called: AtomicBool::new(false),
                handle_overflow,
            }
        }
    }

    #[async_trait]
    impl ContextPlugin for TrackingPlugin {
        fn name(&self) -> &str {
            self.name
        }

        async fn on_pre_completion(
            &self,
            _messages: &mut Vec<Message>,
            _context_window: usize,
        ) -> Result<(), String> {
            self.pre_completion_called.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn on_overflow(
            &self,
            _messages: &mut Vec<Message>,
            _context_window: usize,
        ) -> Result<bool, String> {
            self.overflow_called.store(true, Ordering::SeqCst);
            Ok(self.handle_overflow)
        }

        async fn on_post_turn(
            &self,
            _messages: &[Message],
            _response: &str,
        ) -> Result<(), String> {
            self.post_turn_called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A plugin that counts how many messages it removed in on_pre_completion.
    struct FilterPlugin {
        removed_count: AtomicUsize,
    }

    impl FilterPlugin {
        fn new() -> Self {
            Self {
                removed_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ContextPlugin for FilterPlugin {
        fn name(&self) -> &str {
            "filter-plugin"
        }

        async fn on_pre_completion(
            &self,
            messages: &mut Vec<Message>,
            _context_window: usize,
        ) -> Result<(), String> {
            // Remove all messages containing "remove-me"
            let before = messages.len();
            messages.retain(|m| !m.content.text_content().contains("remove-me"));
            self.removed_count
                .store(before - messages.len(), Ordering::SeqCst);
            Ok(())
        }

        async fn on_overflow(
            &self,
            _messages: &mut Vec<Message>,
            _context_window: usize,
        ) -> Result<bool, String> {
            Ok(false)
        }
    }

    // -----------------------------------------------------------------------
    // DefaultContextPlugin tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_default_plugin_no_overflow() {
        let plugin = DefaultContextPlugin::new("system".to_string(), vec![]);
        let mut msgs = make_messages(2, 100);
        let handled = plugin.on_overflow(&mut msgs, 200_000).await.unwrap();
        // No overflow => not handled
        assert!(!handled);
    }

    #[tokio::test]
    async fn test_default_plugin_handles_overflow() {
        let plugin = DefaultContextPlugin::new("system".to_string(), vec![]);
        // Many large messages with a small context window to trigger overflow
        let mut msgs = make_messages(30, 200);
        let handled = plugin.on_overflow(&mut msgs, 1000).await.unwrap();
        assert!(handled);
        // Messages should have been trimmed
        assert!(msgs.len() < 30);
    }

    #[tokio::test]
    async fn test_default_plugin_pre_completion_is_noop() {
        let plugin = DefaultContextPlugin::new("system".to_string(), vec![]);
        let mut msgs = make_messages(5, 50);
        let original_len = msgs.len();
        plugin.on_pre_completion(&mut msgs, 200_000).await.unwrap();
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test]
    async fn test_default_plugin_name() {
        let plugin = DefaultContextPlugin::new("sys".to_string(), vec![]);
        assert_eq!(plugin.name(), "default-overflow-recovery");
    }

    // -----------------------------------------------------------------------
    // ContextPluginRegistry tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_registry_empty() {
        let registry = ContextPluginRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        let mut msgs = make_messages(2, 50);
        // All operations should be no-ops on empty registry
        registry.run_pre_completion(&mut msgs, 200_000).await.unwrap();
        let handled = registry.run_overflow(&mut msgs, 200_000).await.unwrap();
        assert!(!handled);
        registry.run_post_turn(&msgs, "response").await;
    }

    #[tokio::test]
    async fn test_registry_pre_completion_calls_all() {
        let p1 = Arc::new(TrackingPlugin::new("p1", false));
        let p2 = Arc::new(TrackingPlugin::new("p2", false));

        let mut registry = ContextPluginRegistry::new();
        registry.register(p1.clone());
        registry.register(p2.clone());
        assert_eq!(registry.len(), 2);

        let mut msgs = make_messages(3, 50);
        registry.run_pre_completion(&mut msgs, 200_000).await.unwrap();

        assert!(p1.pre_completion_called.load(Ordering::SeqCst));
        assert!(p2.pre_completion_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_registry_overflow_short_circuits() {
        // p1 does NOT handle overflow, p2 does
        let p1 = Arc::new(TrackingPlugin::new("p1", false));
        let p2 = Arc::new(TrackingPlugin::new("p2", true));
        let p3 = Arc::new(TrackingPlugin::new("p3", true));

        let mut registry = ContextPluginRegistry::new();
        registry.register(p1.clone());
        registry.register(p2.clone());
        registry.register(p3.clone());

        let mut msgs = make_messages(3, 50);
        let handled = registry.run_overflow(&mut msgs, 200_000).await.unwrap();
        assert!(handled);

        // p1 was called but didn't handle
        assert!(p1.overflow_called.load(Ordering::SeqCst));
        // p2 handled it
        assert!(p2.overflow_called.load(Ordering::SeqCst));
        // p3 should NOT have been called (short-circuit)
        assert!(!p3.overflow_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_registry_post_turn_calls_all() {
        let p1 = Arc::new(TrackingPlugin::new("p1", false));
        let p2 = Arc::new(TrackingPlugin::new("p2", false));

        let mut registry = ContextPluginRegistry::new();
        registry.register(p1.clone());
        registry.register(p2.clone());

        let msgs = make_messages(2, 50);
        registry.run_post_turn(&msgs, "test response").await;

        assert!(p1.post_turn_called.load(Ordering::SeqCst));
        assert!(p2.post_turn_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_registry_filter_plugin_modifies_messages() {
        let filter = Arc::new(FilterPlugin::new());
        let mut registry = ContextPluginRegistry::new();
        registry.register(filter.clone());

        let mut msgs = vec![
            Message::user("keep this"),
            Message::user("remove-me please"),
            Message::user("also keep"),
            Message::user("remove-me too"),
        ];

        registry.run_pre_completion(&mut msgs, 200_000).await.unwrap();

        assert_eq!(msgs.len(), 2);
        assert_eq!(filter.removed_count.load(Ordering::SeqCst), 2);
        assert_eq!(msgs[0].content.text_content(), "keep this");
        assert_eq!(msgs[1].content.text_content(), "also keep");
    }

    #[tokio::test]
    async fn test_registry_overflow_none_handle() {
        let p1 = Arc::new(TrackingPlugin::new("p1", false));
        let p2 = Arc::new(TrackingPlugin::new("p2", false));

        let mut registry = ContextPluginRegistry::new();
        registry.register(p1.clone());
        registry.register(p2.clone());

        let mut msgs = make_messages(3, 50);
        let handled = registry.run_overflow(&mut msgs, 200_000).await.unwrap();
        assert!(!handled);

        // Both were called since neither handled it
        assert!(p1.overflow_called.load(Ordering::SeqCst));
        assert!(p2.overflow_called.load(Ordering::SeqCst));
    }

    /// Post-turn errors are logged but don't stop the chain.
    #[tokio::test]
    async fn test_registry_post_turn_error_continues() {
        struct FailingPlugin;

        #[async_trait]
        impl ContextPlugin for FailingPlugin {
            fn name(&self) -> &str {
                "failing"
            }
            async fn on_pre_completion(
                &self,
                _messages: &mut Vec<Message>,
                _context_window: usize,
            ) -> Result<(), String> {
                Ok(())
            }
            async fn on_overflow(
                &self,
                _messages: &mut Vec<Message>,
                _context_window: usize,
            ) -> Result<bool, String> {
                Ok(false)
            }
            async fn on_post_turn(
                &self,
                _messages: &[Message],
                _response: &str,
            ) -> Result<(), String> {
                Err("intentional test error".to_string())
            }
        }

        let after = Arc::new(TrackingPlugin::new("after-fail", false));

        let mut registry = ContextPluginRegistry::new();
        registry.register(Arc::new(FailingPlugin));
        registry.register(after.clone());

        let msgs = make_messages(2, 50);
        // Should not panic; error from FailingPlugin is logged, after-fail still called
        registry.run_post_turn(&msgs, "response").await;
        assert!(after.post_turn_called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_default_registry() {
        let registry = ContextPluginRegistry::default();
        assert!(registry.is_empty());
    }
}
