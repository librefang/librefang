//! #7957 — the vision gate, tested at the two places it is actually wired.
//!
//! `redact_images_for_text_only` has unit coverage in `utilities.rs`, but a correct redactor is not
//! the property that broke. What broke was the *decision* to call it, taken separately in
//! `agent_loop/mod.rs` and in `run_streaming.rs`, and CLAUDE.md's warning applies exactly here: two
//! copies of one gate drift silently, and the streaming copy is the one the WebUI and every channel
//! bridge run. So these tests drive `run_agent_loop` and `run_agent_loop_streaming` end to end with
//! an image attached and a mock kernel that reports each `VisionSupport` variant, and assert on the
//! `CompletionRequest` the driver was actually handed.

use super::*;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
use librefang_kernel_handle::ApiAuthSnapshot;
use librefang_types::message::{ContentBlock, MessageContent, StopReason, TokenUsage};
use librefang_types::model_catalog::VisionSupport;
use std::sync::Mutex;

/// A model id an operator would give a LiteLLM deployment. It matches none of the vision name
/// heuristics (`llava`, `vision`, `-vl-`, `moondream`, …), which is the premise of #7957.
const OPERATOR_ALIAS: &str = "team-default";

/// Driver that answers immediately and keeps every request it was handed.
struct RecordingDriver {
    seen: Mutex<Vec<CompletionRequest>>,
}

impl RecordingDriver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }

    /// Every content block, from every message, of the first request the loop built.
    fn first_request_blocks(&self) -> Vec<ContentBlock> {
        let seen = self.seen.lock().unwrap();
        let first = seen.first().expect("the loop must have called the driver");
        first
            .messages
            .iter()
            .filter_map(|m| match &m.content {
                MessageContent::Blocks(blocks) => Some(blocks.clone()),
                MessageContent::Text(_) => None,
            })
            .flatten()
            .collect()
    }

    fn sent_an_image(&self) -> bool {
        self.first_request_blocks().iter().any(|b| {
            matches!(
                b,
                ContentBlock::Image { .. } | ContentBlock::ImageFile { .. }
            )
        })
    }

    fn placeholder_texts(&self) -> Vec<String> {
        self.first_request_blocks()
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } if text.starts_with("[image omitted") => {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl LlmDriver for RecordingDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.seen.lock().unwrap().push(request);
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "I can see it.".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 5,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// Kernel handle whose only opinion is the one the gate reads.
///
/// Every other role trait takes its default impl, so a future field on `CatalogQuery` cannot make
/// this stub silently claim something it was never told.
struct VisionKernel {
    support: VisionSupport,
}

impl VisionKernel {
    fn arc(support: VisionSupport) -> Arc<dyn librefang_kernel_handle::KernelHandle> {
        Arc::new(Self { support })
    }
}

impl librefang_kernel_handle::CatalogQuery for VisionKernel {
    fn vision_support_for(&self, _model: &str) -> VisionSupport {
        self.support
    }
}

#[async_trait]
impl librefang_kernel_handle::AgentControl for VisionKernel {
    async fn spawn_agent(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn send_to_agent(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn list_agents(&self) -> Vec<librefang_kernel_handle::AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn find_agents(&self, _: &str) -> Vec<librefang_kernel_handle::AgentInfo> {
        vec![]
    }
}

impl librefang_kernel_handle::MemoryAccess for VisionKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::TaskQueue for VisionKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &librefang_kernel_handle::TaskPostOptions,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_claim(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_complete(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_get(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_update_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::EventBus for VisionKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::KnowledgeGraph for VisionKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
        _: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
        _: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
        _: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not used".into())
    }
}

impl librefang_kernel_handle::ApiAuth for VisionKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for VisionKernel {
    fn inject_attachment_blocks(
        &self,
        _: librefang_types::agent::AgentId,
        _: librefang_types::agent::SessionId,
        _: Vec<ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::WikiAccess for VisionKernel {}
impl librefang_kernel_handle::CronControl for VisionKernel {}
impl librefang_kernel_handle::ApprovalGate for VisionKernel {}
impl librefang_kernel_handle::HandsControl for VisionKernel {}
impl librefang_kernel_handle::A2ARegistry for VisionKernel {}
impl librefang_kernel_handle::ChannelSender for VisionKernel {}
impl librefang_kernel_handle::PromptStore for VisionKernel {}
impl librefang_kernel_handle::WorkflowRunner for VisionKernel {}
impl librefang_kernel_handle::GoalControl for VisionKernel {}
impl librefang_kernel_handle::ToolPolicy for VisionKernel {}
impl librefang_kernel_handle::AcpFsBridge for VisionKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for VisionKernel {}

fn manifest_on(model: &str) -> AgentManifest {
    let mut manifest = super::integration::test_manifest();
    manifest.model.provider = "litellm".to_string();
    manifest.model.model = model.to_string();
    manifest
}

fn blank_session() -> Session {
    Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        parent_session_id: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    }
}

fn one_image() -> Vec<ContentBlock> {
    vec![ContentBlock::ImageFile {
        media_type: "image/png".to_string(),
        path: "/tmp/librefang-test/receipt.png".to_string(),
    }]
}

/// Drive the non-streaming entry point with one attached image.
async fn run_non_streaming(support: Option<VisionSupport>) -> Arc<RecordingDriver> {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = blank_session();
    let manifest = manifest_on(OPERATOR_ALIAS);
    let recorder = RecordingDriver::new();
    let driver: Arc<dyn LlmDriver> = recorder.clone();

    run_agent_loop(
        &manifest,
        "What does this say?",
        &mut session,
        &memory,
        driver,
        &[],
        support.map(VisionKernel::arc),
        None,
        None,
        None,
        None,
        None,
        None,
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        Some(one_image()),
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("the loop must complete");
    recorder
}

/// Drive the streaming entry point with one attached image.
async fn run_streaming(support: Option<VisionSupport>) -> Arc<RecordingDriver> {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = blank_session();
    let manifest = manifest_on(OPERATOR_ALIAS);
    let recorder = RecordingDriver::new();
    let driver: Arc<dyn LlmDriver> = recorder.clone();
    let (stream_tx, mut stream_rx) = mpsc::channel(64);
    // Drain the stream so the driver's `tx.send` never blocks on a full channel.
    let drain = tokio::spawn(async move { while stream_rx.recv().await.is_some() {} });

    run_agent_loop_streaming(
        &manifest,
        "What does this say?",
        &mut session,
        &memory,
        driver,
        &[],
        support.map(VisionKernel::arc),
        stream_tx,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        Some(one_image()),
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("the streaming loop must complete");
    drop(drain);
    recorder
}

// ---------------------------------------------------------------------------
// The bug: a declared-vision gateway model whose name matches no heuristic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn declared_vision_keeps_the_image_on_the_non_streaming_path() {
    let recorder = run_non_streaming(Some(VisionSupport::Supported)).await;
    assert!(
        recorder.sent_an_image(),
        "the gateway declares vision support for `{OPERATOR_ALIAS}`; the request must carry the image"
    );
    assert!(recorder.placeholder_texts().is_empty());
}

#[tokio::test]
async fn declared_vision_keeps_the_image_on_the_streaming_path() {
    let recorder = run_streaming(Some(VisionSupport::Supported)).await;
    assert!(
        recorder.sent_an_image(),
        "the streaming gate must agree with the non-streaming one"
    );
    assert!(recorder.placeholder_texts().is_empty());
}

// ---------------------------------------------------------------------------
// Unknown fails open — the catalog-hit path is no longer more confident than
// the catalog-miss path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_capability_fails_open_on_the_non_streaming_path() {
    let recorder = run_non_streaming(Some(VisionSupport::Unknown)).await;
    assert!(
        recorder.sent_an_image(),
        "an unproven capability must never cost the user their image: if the model really is \
         text-only the provider answers HTTP 400, which is a failure someone can act on"
    );
}

#[tokio::test]
async fn unknown_capability_fails_open_on_the_streaming_path() {
    let recorder = run_streaming(Some(VisionSupport::Unknown)).await;
    assert!(recorder.sent_an_image());
}

#[tokio::test]
async fn no_kernel_handle_still_fails_open_on_both_paths() {
    assert!(run_non_streaming(None).await.sent_an_image());
    assert!(run_streaming(None).await.sent_an_image());
}

// ---------------------------------------------------------------------------
// A declared text-only model is still redacted — #6010 must keep working
// ---------------------------------------------------------------------------

#[tokio::test]
async fn declared_text_only_still_strips_the_image_on_the_non_streaming_path() {
    let recorder = run_non_streaming(Some(VisionSupport::Unsupported)).await;
    assert!(
        !recorder.sent_an_image(),
        "a model declared text-only rejects `image_url` parts with HTTP 400; #6010's redaction must survive"
    );
    let placeholders = recorder.placeholder_texts();
    assert_eq!(placeholders.len(), 1, "got {placeholders:?}");
    assert!(
        placeholders[0].contains(OPERATOR_ALIAS)
            && placeholders[0].contains("/tmp/librefang-test/receipt.png"),
        "the placeholder must name the model and keep the file path: {placeholders:?}"
    );
}

#[tokio::test]
async fn declared_text_only_still_strips_the_image_on_the_streaming_path() {
    let recorder = run_streaming(Some(VisionSupport::Unsupported)).await;
    assert!(!recorder.sent_an_image());
    assert_eq!(recorder.placeholder_texts().len(), 1);
}

/// The silence half of #7957: a redaction that happens must be observable.
///
/// The assertion is on the `tracing` event, not on the placeholder, because the placeholder goes to
/// the *model* and the operator never sees it. Watching a vision-capable model answer from a
/// filename, the reporter had nothing to grep for.
#[tokio::test]
async fn a_strip_emits_a_warn_naming_the_model_and_the_reason() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tracing::field::{Field, Visit};
    use tracing::subscriber::with_default;
    use tracing::Level;

    #[derive(Default)]
    struct WarnSpy {
        saw_model: Arc<AtomicBool>,
        saw_count: Arc<AtomicBool>,
        saw_reason: Arc<AtomicBool>,
    }

    struct Recorder<'a>(&'a WarnSpy);

    impl Visit for Recorder<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            let rendered = format!("{value:?}");
            match field.name() {
                "model" if rendered.contains(OPERATOR_ALIAS) => {
                    self.0.saw_model.store(true, Ordering::Relaxed);
                }
                "images_redacted" if rendered == "1" => {
                    self.0.saw_count.store(true, Ordering::Relaxed);
                }
                "message" if rendered.contains("text-only") => {
                    self.0.saw_reason.store(true, Ordering::Relaxed);
                }
                _ => {}
            }
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            if field.name() == "images_redacted" && value == 1 {
                self.0.saw_count.store(true, Ordering::Relaxed);
            }
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "model" && value.contains(OPERATOR_ALIAS) {
                self.0.saw_model.store(true, Ordering::Relaxed);
            }
        }
    }

    impl tracing::Subscriber for WarnSpy {
        fn enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
            *meta.level() <= Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            event.record(&mut Recorder(self));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    let spy = WarnSpy::default();
    let (saw_model, saw_count, saw_reason) = (
        spy.saw_model.clone(),
        spy.saw_count.clone(),
        spy.saw_reason.clone(),
    );

    // `redact_images_for_text_only` is where the WARN lives, and it is the one function both gate
    // sites call — testing it directly keeps the assertion off the loop's own log traffic.
    with_default(spy, || {
        let messages = vec![Message::user_with_blocks(one_image())];
        let out = super::super::redact_images_for_text_only(messages, OPERATOR_ALIAS);
        assert_eq!(out.len(), 1);
    });

    assert!(
        saw_model.load(Ordering::Relaxed),
        "the WARN must name the model whose images were dropped"
    );
    assert!(
        saw_count.load(Ordering::Relaxed),
        "the WARN must say how many blocks were replaced"
    );
    assert!(
        saw_reason.load(Ordering::Relaxed),
        "the WARN must say why — an operator greps for the reason, not for the model id"
    );
}

/// The no-image case stays silent: a turn that never carried a picture has nothing to warn about,
/// and a `WARN` per text turn would train operators to ignore the one that matters.
#[tokio::test]
async fn no_images_means_no_warning() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSpy(Arc<AtomicUsize>);

    impl tracing::Subscriber for CountingSpy {
        fn enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
            *meta.level() <= tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, _: &tracing::Event<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    let count = Arc::new(AtomicUsize::new(0));
    tracing::subscriber::with_default(CountingSpy(count.clone()), || {
        let messages = vec![Message::user("no pictures here")];
        super::super::redact_images_for_text_only(messages, OPERATOR_ALIAS);
    });
    assert_eq!(count.load(Ordering::Relaxed), 0);
}
