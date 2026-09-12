//! Capability routing for inbound media the agent's own model cannot process.
//!
//! ## The failure this replaces
//!
//! When a text-only model was handed an image, `redact_images_for_text_only`
//! swapped the pixels for `[image omitted: model X has no vision support. The
//! image file is on disk at <path>]` and the turn continued. The model was
//! then holding a filename and a browser, and it *described the picture
//! anyway* — a hallucination shaped exactly like a correct answer, because
//! nothing in the transcript said the description was invented.
//!
//! The channel bridge had already solved this for Telegram (`bridge.rs`
//! describes the photo with `MediaEngine` and prepends the text). The API and
//! dashboard entry paths had not, so the same agent behaved differently
//! depending on which door the image came through.
//!
//! ## What happens instead
//!
//! Before the user turn is pushed into history, an image bound for a
//! vision-less model is sent to whichever provider the resolved
//! `[capabilities]` block nominates for `image_understanding`, and the
//! resulting text is inserted next to the image block. The redaction gate then
//! strips the pixels exactly as before — but the description survives, because
//! redaction only rewrites `Image` / `ImageFile` blocks.
//!
//! Four properties this file is responsible for:
//!
//! 1. **Conditional.** A model that *can* see is never charged for a
//!    description; the enrichment is skipped entirely.
//! 2. **Once per turn.** The work happens before the loop, not inside it, so
//!    a ten-iteration tool-use turn does not pay for ten descriptions.
//! 3. **No double-describe.** An image the channel bridge already described
//!    arrives carrying its description block in the slot immediately before
//!    it, and is left alone — per image, so a mixed message still describes
//!    the one that arrived bare.
//! 4. **Never fatal.** A provider failure, or one that does not answer within
//!    [`DESCRIPTION_TIMEOUT`], degrades to `[Image description unavailable]` —
//!    which still tells the model it is not looking at the image — rather than
//!    dropping the turn or holding the session open.

use librefang_types::media::{
    image_description_block_text, is_image_description_text, IMAGE_DESCRIPTION_UNAVAILABLE,
};
use librefang_types::message::ContentBlock;

/// Ceiling on a single description call, mirroring the channel bridge's
/// `INBOUND_DESCRIPTION_TIMEOUT`.
///
/// Vision APIs answer in 2-8s, but the loop below is sequential and
/// `MediaEngine::describe_image` waits on a semaphore permit *before* it
/// issues a request whose own ceiling is 60s.
/// Without a bound here, one hung provider plus a queue of other media work
/// pins the turn — and the operator's message has not reached the agent's own
/// model yet — so a six-image upload could hold a session for minutes.
/// On elapse the call degrades through the same path as any other provider
/// failure: the model is told the image is unavailable rather than left to
/// invent it.
pub(crate) const DESCRIPTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// An image block reduced to what a describer needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageRef<'a> {
    /// `ContentBlock::ImageFile` — pixels live on disk.
    File { path: &'a str, media_type: &'a str },
    /// `ContentBlock::Image` — inline base64 payload.
    Inline { data: &'a str, media_type: &'a str },
}

impl<'a> ImageRef<'a> {
    fn from_block(block: &'a ContentBlock) -> Option<Self> {
        match block {
            ContentBlock::ImageFile { path, media_type } => Some(ImageRef::File {
                path: path.as_str(),
                media_type: media_type.as_str(),
            }),
            ContentBlock::Image { data, media_type } => Some(ImageRef::Inline {
                data: data.as_str(),
                media_type: media_type.as_str(),
            }),
            _ => None,
        }
    }
}

/// "Turn this image into words."
///
/// A trait rather than a direct `MediaEngine` call so the ordering, the
/// dedupe rule and the failure behaviour above can be unit-tested without a
/// live vision provider — those are the parts that regress silently.
#[async_trait::async_trait]
pub(crate) trait ImageDescriber: Send + Sync {
    async fn describe(&self, image: ImageRef<'_>) -> Result<String, String>;
}

/// `true` when the image at `idx` already carries its description.
///
/// Adjacency, not "somewhere in the message": the channel bridge inserts each
/// description in the slot immediately before the image it describes, so that
/// slot is the only one that means anything.
/// Scanning the whole message instead reads *user-controlled* text — a caption
/// reading `[Image description: see attached]`, or a forwarded chat log that
/// happens to quote the line — and would skip describing a picture nobody had
/// described, putting the agent back to answering from a filename.
/// It also mis-handles the mixed case, where one image arrived described and a
/// second did not.
fn image_at_is_already_described(blocks: &[ContentBlock], idx: usize) -> bool {
    idx > 0
        && matches!(
            &blocks[idx - 1],
            ContentBlock::Text { text, .. } if is_image_description_text(text)
        )
}

/// `true` when every image in `blocks` already carries its description, so
/// there is nothing for this hop to do and no provider call to pay for.
pub(crate) fn blocks_already_describe_images(blocks: &[ContentBlock]) -> bool {
    let mut saw_image = false;
    for (idx, block) in blocks.iter().enumerate() {
        if ImageRef::from_block(block).is_some() {
            saw_image = true;
            if !image_at_is_already_described(blocks, idx) {
                return false;
            }
        }
    }
    saw_image
}

/// `true` when `blocks` contains something a vision-less model cannot read.
pub(crate) fn blocks_contain_images(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|b| ImageRef::from_block(b).is_some())
}

/// Insert a description block immediately before every image block.
///
/// Before, not after, so the model reads the words and *then* meets the
/// placeholder that redaction leaves behind — the same order the channel
/// bridge produces.
pub(crate) async fn enrich_blocks_with_image_descriptions(
    blocks: Vec<ContentBlock>,
    describer: &dyn ImageDescriber,
) -> Vec<ContentBlock> {
    if !blocks_contain_images(&blocks) || blocks_already_describe_images(&blocks) {
        return blocks;
    }

    // Decided before the blocks are consumed, because the answer depends on
    // the *neighbour* of each image and `out` no longer has those neighbours
    // in their original positions once descriptions start being inserted.
    let needs_description: Vec<bool> = blocks
        .iter()
        .enumerate()
        .map(|(idx, b)| {
            ImageRef::from_block(b).is_some() && !image_at_is_already_described(&blocks, idx)
        })
        .collect();

    let mut out = Vec::with_capacity(blocks.len() * 2);
    for (idx, block) in blocks.into_iter().enumerate() {
        // Per image, not per message: an image the bridge already described is
        // left alone, and a second undescribed image in the same message is
        // still described.
        if let Some(image) = ImageRef::from_block(&block).filter(|_| needs_description[idx]) {
            let described =
                match tokio::time::timeout(DESCRIPTION_TIMEOUT, describer.describe(image)).await {
                    Ok(result) => result,
                    Err(_) => Err(format!(
                        "no answer within {}s",
                        DESCRIPTION_TIMEOUT.as_secs()
                    )),
                };
            let text = match described {
                Ok(description) if !description.trim().is_empty() => {
                    image_description_block_text(&description)
                }
                Ok(_) => {
                    tracing::warn!(
                        "Image description returned empty text for a model with no vision \
                         support; the agent will be told the image is unavailable rather \
                         than being left to guess"
                    );
                    IMAGE_DESCRIPTION_UNAVAILABLE.to_string()
                }
                Err(reason) => {
                    tracing::warn!(
                        error = %reason,
                        "Image description failed for a model with no vision support"
                    );
                    IMAGE_DESCRIPTION_UNAVAILABLE.to_string()
                }
            };
            out.push(ContentBlock::Text {
                text,
                provider_metadata: None,
            });
        }
        out.push(block);
    }
    out
}

/// Why a turn is, or is not, sent for description.
///
/// Split out of [`describe_images_for_text_only_model`] as a pure function so
/// the gate itself is testable: whether a *vision-capable* model is charged for
/// a description is a cost decision, and "it silently started describing
/// everything" is exactly the kind of regression that never shows up in a
/// functional test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutingDecision {
    Describe,
    /// Skipped, with the reason — used for the debug log and asserted in tests.
    Skip(&'static str),
}

/// Inputs to the gate, named so a call site cannot transpose two booleans.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RoutingInputs {
    pub has_images: bool,
    pub already_described: bool,
    pub model_supports_vision: bool,
    pub engine_available: bool,
    pub description_enabled: bool,
}

pub(crate) fn routing_decision(inputs: RoutingInputs) -> RoutingDecision {
    if !inputs.has_images {
        return RoutingDecision::Skip("no images in turn");
    }
    if inputs.already_described {
        return RoutingDecision::Skip("images already described upstream");
    }
    if inputs.model_supports_vision {
        return RoutingDecision::Skip("model supports vision");
    }
    if !inputs.engine_available {
        return RoutingDecision::Skip("no media engine wired");
    }
    if !inputs.description_enabled {
        return RoutingDecision::Skip("[media] image_description is off");
    }
    RoutingDecision::Describe
}

// ---------------------------------------------------------------------------
// MediaEngine wiring
// ---------------------------------------------------------------------------

#[cfg(feature = "media")]
mod engine {
    use super::{ImageDescriber, ImageRef};
    use crate::media_understanding::MediaEngine;
    use librefang_types::media::{MediaAttachment, MediaSource, MediaType};

    /// Adapts a capability-routed [`MediaEngine`] to [`ImageDescriber`].
    pub(crate) struct MediaEngineDescriber {
        pub(crate) engine: MediaEngine,
    }

    #[async_trait::async_trait]
    impl ImageDescriber for MediaEngineDescriber {
        async fn describe(&self, image: ImageRef<'_>) -> Result<String, String> {
            let attachment = match image {
                ImageRef::File { path, media_type } => {
                    // `MediaAttachment::validate` enforces the size ceiling, so
                    // the real on-disk size has to be measured rather than
                    // guessed — a stat failure here is also the cheapest way to
                    // notice the file went away between download and dispatch.
                    let size_bytes = tokio::fs::metadata(path)
                        .await
                        .map_err(|e| format!("stat image '{path}' failed: {e}"))?
                        .len();
                    MediaAttachment {
                        media_type: MediaType::Image,
                        mime_type: media_type.to_string(),
                        source: MediaSource::FilePath {
                            path: path.to_string(),
                        },
                        size_bytes,
                    }
                }
                ImageRef::Inline { data, media_type } => MediaAttachment {
                    media_type: MediaType::Image,
                    mime_type: media_type.to_string(),
                    source: MediaSource::Base64 {
                        data: data.to_string(),
                        mime_type: media_type.to_string(),
                    },
                    // Base64 expands 3 bytes into 4 characters; the decoded
                    // size is what the ceiling is expressed in.
                    size_bytes: (data.len() as u64) * 3 / 4,
                },
            };
            self.engine
                .describe_image(&attachment)
                .await
                .map(|u| u.description)
        }
    }
}

/// Describe the images in a user turn when — and only when — the agent's own
/// model cannot see them.
///
/// Returns the blocks unchanged (and does no work, and spends no money) when
/// any of these holds:
///
/// - the turn carries no images;
/// - the model supports vision, per the catalog (`vision_support_for` fails
///   open, so an unknown model keeps receiving pixels);
/// - no `MediaEngine` is wired, or the operator turned
///   `[media] image_description` off;
/// - the images already carry descriptions from the channel bridge.
///
/// The capability chain applied to the describing call is
/// `agent [capabilities]` > kernel-global `[capabilities]` (already folded
/// into the engine at boot) > `[media] image_provider` > env auto-detection.
#[cfg(feature = "media")]
pub(super) async fn describe_images_for_text_only_model(
    blocks: Option<Vec<ContentBlock>>,
    manifest: &librefang_types::agent::AgentManifest,
    kernel: Option<&std::sync::Arc<dyn crate::kernel_handle::KernelHandle>>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
) -> Option<Vec<ContentBlock>> {
    let blocks = blocks?;
    let has_images = blocks_contain_images(&blocks);
    let api_model = super::strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
    // `vision_support_for` fails open: no kernel handle wired, or a model the
    // catalog does not know, resolves to `VisionSupport::Unknown` and keeps the
    // image rather than being needlessly described. Only consulted when there is
    // an image to gate — otherwise the catalog lookup is pure overhead on every
    // text turn.
    let model_supports_vision = !has_images
        || kernel
            .map(|k| k.vision_support_for(&api_model))
            .unwrap_or(librefang_types::model_catalog::VisionSupport::Unknown)
            .allows_images();

    let decision = routing_decision(RoutingInputs {
        has_images,
        already_described: blocks_already_describe_images(&blocks),
        model_supports_vision,
        engine_available: media_engine.is_some(),
        description_enabled: media_engine.is_some_and(|e| e.image_description_enabled()),
    });

    let (RoutingDecision::Describe, Some(engine)) = (decision, media_engine) else {
        if let RoutingDecision::Skip(reason) = decision {
            if has_images {
                tracing::debug!(
                    agent = %manifest.name,
                    model = %api_model,
                    reason,
                    "Inbound image not routed for description"
                );
            }
        }
        return Some(blocks);
    };

    tracing::info!(
        agent = %manifest.name,
        model = %api_model,
        "Model has no vision support — routing the inbound image to the configured \
         image_understanding capability instead of dropping it"
    );
    let describer = engine::MediaEngineDescriber {
        engine: engine.with_capability_routing(&manifest.capabilities.routing),
    };
    Some(enrich_blocks_with_image_descriptions(blocks, &describer).await)
}

/// No-media build: the turn is passed through untouched and the existing
/// redaction placeholder remains the only thing the model sees.
#[cfg(not(feature = "media"))]
pub(super) async fn describe_images_for_text_only_model(
    blocks: Option<Vec<ContentBlock>>,
    _manifest: &librefang_types::agent::AgentManifest,
    _kernel: Option<&std::sync::Arc<dyn crate::kernel_handle::KernelHandle>>,
    _media_engine: Option<&crate::media_understanding::MediaEngine>,
) -> Option<Vec<ContentBlock>> {
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct StubDescriber {
        result: Result<String, String>,
        calls: Mutex<Vec<String>>,
    }

    impl StubDescriber {
        fn ok(text: &str) -> Self {
            Self {
                result: Ok(text.to_string()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn err(reason: &str) -> Self {
            Self {
                result: Err(reason.to_string()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl ImageDescriber for StubDescriber {
        async fn describe(&self, image: ImageRef<'_>) -> Result<String, String> {
            let label = match image {
                ImageRef::File { path, .. } => path.to_string(),
                ImageRef::Inline { data, .. } => format!("inline:{}", data.len()),
            };
            self.calls.lock().unwrap().push(label);
            self.result.clone()
        }
    }

    fn image_file(path: &str) -> ContentBlock {
        ContentBlock::ImageFile {
            media_type: "image/png".to_string(),
            path: path.to_string(),
        }
    }

    fn text(t: &str) -> ContentBlock {
        ContentBlock::Text {
            text: t.to_string(),
            provider_metadata: None,
        }
    }

    fn texts(blocks: &[ContentBlock]) -> Vec<String> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The regression guard for the reported bug: a text-only agent handed an
    /// image must end up with the image's *contents* in the transcript, not
    /// just a path it can invent an answer around. Delete the enrichment and
    /// this fails.
    #[tokio::test]
    async fn image_bound_for_a_text_only_model_gains_a_description_block() {
        let stub = StubDescriber::ok("A hand-drawn circuit diagram with three resistors.");
        let out = enrich_blocks_with_image_descriptions(
            vec![text("what is this?"), image_file("/tmp/photo.png")],
            &stub,
        )
        .await;

        assert_eq!(stub.call_count(), 1);
        let all = texts(&out);
        assert!(
            all.iter()
                .any(|t| t
                    == "[Image description: A hand-drawn circuit diagram with three resistors.]"),
            "expected a description block, got {all:?}"
        );
        // Ordering: description immediately before the image it describes.
        assert!(matches!(out[1], ContentBlock::Text { .. }));
        assert!(matches!(out[2], ContentBlock::ImageFile { .. }));
        // The image block itself is untouched — redaction still owns it.
        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn description_survives_the_text_only_redaction_pass() {
        let stub = StubDescriber::ok("A red bicycle leaning on a wall.");
        let enriched = enrich_blocks_with_image_descriptions(
            vec![text("what is this?"), image_file("/tmp/bike.png")],
            &stub,
        )
        .await;

        let message = librefang_types::message::Message::user_with_blocks(enriched);
        let redacted = super::super::redact_images_for_text_only(vec![message], "deepseek-chat");
        let librefang_types::message::MessageContent::Blocks(blocks) = &redacted[0].content else {
            panic!("expected blocks");
        };
        let all = texts(blocks);
        assert!(
            all.iter()
                .any(|t| t == "[Image description: A red bicycle leaning on a wall.]"),
            "redaction must not eat the description: {all:?}"
        );
        assert!(
            all.iter().any(|t| t.starts_with("[image omitted:")),
            "the pixels must still be redacted: {all:?}"
        );
    }

    #[tokio::test]
    async fn an_image_the_channel_bridge_already_described_is_not_described_again() {
        let stub = StubDescriber::ok("should never be called");
        let blocks = vec![
            text("[Image description: A cat sitting on a mat.]"),
            image_file("/tmp/cat.png"),
        ];
        let out = enrich_blocks_with_image_descriptions(blocks.clone(), &stub).await;
        assert_eq!(
            stub.call_count(),
            0,
            "must not double-bill for a description"
        );
        assert_eq!(texts(&out), texts(&blocks));
    }

    /// The dedupe sniff must not read text the *user* wrote. A caption that
    /// happens to be shaped like a description block — typed, or quoted out of
    /// a forwarded chat log — used to skip the whole message, leaving the
    /// text-only model holding a filename again.
    #[tokio::test]
    async fn a_user_typed_caption_elsewhere_in_the_message_does_not_suppress_the_description() {
        let stub = StubDescriber::ok("A signed contract, page 3.");
        let out = enrich_blocks_with_image_descriptions(
            vec![
                text("[Image description: see attached]"),
                text("what does it say?"),
                image_file("/tmp/contract.png"),
            ],
            &stub,
        )
        .await;

        assert_eq!(
            stub.call_count(),
            1,
            "the image is undescribed — user text must not stand in for a real description"
        );
        assert!(
            texts(&out)
                .iter()
                .any(|t| t == "[Image description: A signed contract, page 3.]"),
            "expected the real description: {out:?}"
        );
    }

    /// Mixed message: the bridge described the first image and the second
    /// arrived bare. Whole-message granularity skipped both.
    #[tokio::test]
    async fn a_second_undescribed_image_is_still_described() {
        let stub = StubDescriber::ok("the second picture");
        let out = enrich_blocks_with_image_descriptions(
            vec![
                text("[Image description: the first picture]"),
                image_file("/tmp/a.png"),
                image_file("/tmp/b.png"),
            ],
            &stub,
        )
        .await;

        assert_eq!(
            stub.call_count(),
            1,
            "exactly the bare image is described, and the described one is not re-billed"
        );
        assert_eq!(
            stub.calls.lock().unwrap().as_slice(),
            ["/tmp/b.png"],
            "the wrong image was sent for description"
        );
        assert_eq!(
            texts(&out),
            vec![
                "[Image description: the first picture]",
                "[Image description: the second picture]"
            ]
        );
    }

    /// A provider that never answers must not hold the turn open. Without the
    /// `tokio::time::timeout`, this test hangs until the harness kills it.
    #[tokio::test(start_paused = true)]
    async fn a_provider_that_never_answers_degrades_to_the_unavailable_marker() {
        struct HangingDescriber;
        #[async_trait::async_trait]
        impl ImageDescriber for HangingDescriber {
            async fn describe(&self, _image: ImageRef<'_>) -> Result<String, String> {
                // Longer than the ceiling by a margin the clock cannot round away.
                tokio::time::sleep(DESCRIPTION_TIMEOUT * 4).await;
                Ok("far too late".to_string())
            }
        }

        let out = enrich_blocks_with_image_descriptions(
            vec![image_file("/tmp/slow.png")],
            &HangingDescriber,
        )
        .await;
        assert_eq!(texts(&out), vec!["[Image description unavailable]"]);
        assert_eq!(out.len(), 2, "the image block is still delivered");
    }

    #[tokio::test]
    async fn a_failed_channel_description_also_suppresses_a_retry() {
        let stub = StubDescriber::ok("should never be called");
        let blocks = vec![
            text("[Image description unavailable]"),
            image_file("/tmp/cat.png"),
        ];
        let out = enrich_blocks_with_image_descriptions(blocks, &stub).await;
        assert_eq!(stub.call_count(), 0);
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn a_provider_failure_degrades_to_the_unavailable_marker() {
        let stub = StubDescriber::err("groq returned 503");
        let out =
            enrich_blocks_with_image_descriptions(vec![image_file("/tmp/x.png")], &stub).await;
        assert_eq!(texts(&out), vec!["[Image description unavailable]"]);
        assert_eq!(out.len(), 2, "the image block is still delivered");
    }

    #[tokio::test]
    async fn an_empty_description_is_reported_as_unavailable_not_as_an_answer() {
        let stub = StubDescriber::ok("   ");
        let out =
            enrich_blocks_with_image_descriptions(vec![image_file("/tmp/x.png")], &stub).await;
        assert_eq!(texts(&out), vec!["[Image description unavailable]"]);
    }

    /// Pins the empty-but-`Ok` arm specifically: the describer *succeeded* but
    /// had nothing to say, and the model must be told the image is unavailable
    /// rather than handed a description block with an empty description inside
    /// it. Collapsing the arm back to `Ok(d) =>` produces
    /// `[Image description: ]` here and fails this test.
    #[tokio::test]
    async fn an_empty_ok_description_does_not_inject_a_blank_text_block() {
        let stub = StubDescriber::ok("");
        let out =
            enrich_blocks_with_image_descriptions(vec![image_file("/tmp/x.png")], &stub).await;

        let blank = texts(&out).into_iter().any(|t| t.trim().is_empty());
        assert!(!blank, "no blank text block may reach the model: {out:?}");
        assert_eq!(
            texts(&out),
            vec!["[Image description unavailable]"],
            "an empty description must read as unavailable, not as an (empty) answer"
        );
        assert_eq!(out.len(), 2, "the image block is still delivered");
    }

    #[tokio::test]
    async fn a_turn_with_no_images_is_returned_untouched_without_calling_the_provider() {
        let stub = StubDescriber::ok("never");
        let blocks = vec![text("just a question")];
        let out = enrich_blocks_with_image_descriptions(blocks.clone(), &stub).await;
        assert_eq!(stub.call_count(), 0);
        assert_eq!(texts(&out), texts(&blocks));
    }

    #[tokio::test]
    async fn every_image_in_a_multi_image_turn_gets_its_own_description() {
        let stub = StubDescriber::ok("a picture");
        let out = enrich_blocks_with_image_descriptions(
            vec![
                text("compare these"),
                image_file("/tmp/a.png"),
                image_file("/tmp/b.png"),
            ],
            &stub,
        )
        .await;
        assert_eq!(stub.call_count(), 2);
        assert_eq!(out.len(), 5);
    }

    // ── The gate ────────────────────────────────────────────────────────

    fn inputs() -> RoutingInputs {
        RoutingInputs {
            has_images: true,
            already_described: false,
            model_supports_vision: false,
            engine_available: true,
            description_enabled: true,
        }
    }

    /// The cost guard. A model that can see must never be billed for a
    /// description — flipping this to unconditional would look correct in
    /// every functional test and double the bill for every vision agent.
    #[test]
    fn a_vision_capable_model_is_never_charged_for_a_description() {
        assert_eq!(
            routing_decision(RoutingInputs {
                model_supports_vision: true,
                ..inputs()
            }),
            RoutingDecision::Skip("model supports vision")
        );
    }

    #[test]
    fn the_gate_describes_only_for_a_text_only_model_with_an_undescribed_image() {
        assert_eq!(routing_decision(inputs()), RoutingDecision::Describe);

        assert_eq!(
            routing_decision(RoutingInputs {
                has_images: false,
                ..inputs()
            }),
            RoutingDecision::Skip("no images in turn")
        );
        assert_eq!(
            routing_decision(RoutingInputs {
                already_described: true,
                ..inputs()
            }),
            RoutingDecision::Skip("images already described upstream")
        );
        assert_eq!(
            routing_decision(RoutingInputs {
                engine_available: false,
                ..inputs()
            }),
            RoutingDecision::Skip("no media engine wired")
        );
        assert_eq!(
            routing_decision(RoutingInputs {
                description_enabled: false,
                ..inputs()
            }),
            RoutingDecision::Skip("[media] image_description is off"),
            "the operator's [media] image_description switch must still govern this path"
        );
    }

    #[tokio::test]
    async fn inline_base64_images_are_described_too() {
        let stub = StubDescriber::ok("a chart");
        let out = enrich_blocks_with_image_descriptions(
            vec![ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "AAAA".to_string(),
            }],
            &stub,
        )
        .await;
        assert_eq!(stub.call_count(), 1);
        assert_eq!(texts(&out), vec!["[Image description: a chart]"]);
    }
}
