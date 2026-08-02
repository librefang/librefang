//! The `MoaDriver` facade.
//!
//! Presents a Mixture-of-Agents preset as a single [`LlmDriver`]. On each
//! completion it (1) flattens the conversation into a blind advisory view,
//! (2) fans out to the enabled advisor models per the preset's cadence,
//! (3) builds a private guidance block from their outputs, (4) appends it to
//! the aggregator's request, and (5) delegates to the aggregator driver.
//!
//! Advisor usage/cost is accounted separately from the aggregator and handed
//! to the kernel via [`MoaDriver::consume_reference_usage`].

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_trait::async_trait;
use librefang_llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
};
use librefang_types::config::{KernelConfig, MoaFanout, MoaPreset, MoaSlot};
use librefang_types::message::{Message, Role, TokenUsage};
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::moa::advisory_view::{build_advisory_view, END_ON_USER_MARKER};
use crate::moa::fanout::{
    effective_reference_temperature, effective_reference_timeout, resolve_slot_driver, run_fanout,
    slot_label, AdvisorResult,
};
use crate::moa::guidance::build_guidance_block;
use crate::moa::privacy::redact_pii;
use crate::moa::progress::MoaProgressEvent;
use crate::model_catalog::ModelCatalog;

/// Capacity of the progress broadcast channel.
const PROGRESS_CHANNEL_CAPACITY: usize = 64;

/// A cached fan-out outcome, keyed by conversation signature.
#[derive(Clone)]
struct CachedFanout {
    results: Vec<AdvisorResult>,
}

/// Advisor outputs plus their cache provenance.
struct Fanout {
    results: Vec<AdvisorResult>,
    /// `true` when produced by a fresh fan-out (cache MISS). A HIT must not
    /// deposit usage/cost or write a trace.
    fresh: bool,
}

/// Turn-scoped cadence state for `EveryN`.
#[derive(Default)]
struct CadenceState {
    /// Hash of the last seen user-turn prefix (detects new user turns).
    last_user_prefix: Option<u64>,
    /// Iteration counter within the current user turn.
    counter: u32,
}

/// The Mixture-of-Agents driver.
pub struct MoaDriver {
    preset_name: String,
    preset: MoaPreset,
    config: Arc<KernelConfig>,
    aggregator_driver: Arc<dyn LlmDriver>,
    aggregator_slot: MoaSlot,
    catalog: Option<ModelCatalog>,
    /// Turn-scoped cadence cache: signature → cached fan-out.
    cache: Mutex<HashMap<u64, CachedFanout>>,
    /// Most recent fan-out (for `EveryN` skip iterations).
    last_fanout: Mutex<Option<CachedFanout>>,
    /// Cadence counter state.
    cadence: Mutex<CadenceState>,
    /// Pending advisor usage/cost for the kernel to consume.
    pending_usage: Mutex<Option<(TokenUsage, f64)>>,
    /// Progress broadcast (kernel-owned receiver lives in the loop).
    progress_tx: Option<broadcast::Sender<MoaProgressEvent>>,
}

impl MoaDriver {
    /// Create a new MoA driver.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        preset_name: String,
        preset: MoaPreset,
        config: Arc<KernelConfig>,
        aggregator_driver: Arc<dyn LlmDriver>,
        aggregator_slot: MoaSlot,
        catalog: Option<ModelCatalog>,
        progress_tx: Option<broadcast::Sender<MoaProgressEvent>>,
    ) -> Self {
        Self {
            preset_name,
            preset,
            config,
            aggregator_driver,
            aggregator_slot,
            catalog,
            cache: Mutex::new(HashMap::new()),
            last_fanout: Mutex::new(None),
            cadence: Mutex::new(CadenceState::default()),
            pending_usage: Mutex::new(None),
            progress_tx,
        }
    }

    /// Create a progress broadcast channel sized for MoA events.
    pub fn progress_channel() -> broadcast::Sender<MoaProgressEvent> {
        broadcast::channel(PROGRESS_CHANNEL_CAPACITY).0
    }

    /// Subscribe to progress events, if broadcasting is enabled.
    pub fn progress_receiver(&self) -> Option<broadcast::Receiver<MoaProgressEvent>> {
        self.progress_tx.as_ref().map(|tx| tx.subscribe())
    }

    /// Take the pending advisor usage/cost, leaving `None` behind.
    pub fn consume_reference_usage(&self) -> Option<(TokenUsage, f64)> {
        self.pending_usage.lock().take()
    }

    /// The aggregator slot that served the last completion.
    pub fn last_aggregator_slot(&self) -> &MoaSlot {
        &self.aggregator_slot
    }

    /// The aggregator driver backing this preset.
    ///
    /// Auxiliary/side tasks unwrap a MoA primary through this so they call the
    /// aggregator directly instead of paying for an advisor fan-out.
    pub fn aggregator_driver(&self) -> Arc<dyn LlmDriver> {
        Arc::clone(&self.aggregator_driver)
    }

    /// Emit a progress event, ignoring the no-receiver case.
    fn emit(&self, event: MoaProgressEvent) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(event);
        }
    }

    /// Enabled advisor slots from the preset.
    fn enabled_advisors(&self) -> Vec<MoaSlot> {
        self.preset
            .reference_models
            .iter()
            .filter(|slot| slot.enabled)
            .cloned()
            .collect()
    }

    /// Run the full MoA pipeline and return the aggregator request + advisor
    /// accounting. Shared by [`complete`](LlmDriver::complete) and
    /// [`stream`](LlmDriver::stream).
    async fn prepare_aggregator_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionRequest, LlmError> {
        let advisor_slots = self.enabled_advisors();

        // Resolve advisor drivers; drop slots whose driver fails to init.
        let advisors: Vec<(MoaSlot, Arc<dyn LlmDriver>)> = advisor_slots
            .iter()
            .filter_map(|slot| {
                resolve_slot_driver(slot, &self.config, self.catalog.as_ref())
                    .map(|driver| (slot.clone(), driver))
            })
            .collect();

        let advisory_view = build_advisory_view(&request.messages);

        let Fanout { results, fresh } = if advisors.is_empty() {
            Fanout {
                results: Vec::new(),
                fresh: false,
            }
        } else {
            self.maybe_run_fanout(&advisors, &advisory_view).await
        };

        // Deposit advisor usage/cost and write a trace on a cache MISS only.
        // A HIT reuses stored outputs: it must add no spend and no trace.
        // Accumulate rather than overwrite — the loop calls us once per
        // iteration, and the kernel picks the total up once at turn end, so
        // an assignment would silently drop every earlier fan-out's spend.
        if fresh && !results.is_empty() {
            let usage = sum_usage(&results);
            let cost: f64 = results.iter().map(|r| r.cost).sum();
            let mut pending = self.pending_usage.lock();
            match pending.as_mut() {
                Some((acc_usage, acc_cost)) => {
                    add_usage(acc_usage, &usage);
                    *acc_cost += cost;
                }
                None => *pending = Some((usage, cost)),
            }
            drop(pending);
            self.maybe_persist_trace(&results);
        }

        // Build + attach guidance.
        let aggregator_label = slot_label(&self.aggregator_slot);
        let guidance = build_guidance_block(
            &self.preset_name,
            &aggregator_label,
            &results,
            self.preset.degraded_reference_policy,
        );

        let mut messages: Vec<Message> = (*request.messages).clone();
        if let Some(mut block) = guidance {
            if self.config.moa.privacy_filter.redacts_advisor_text() {
                block = redact_pii(&block);
            }
            crate::moa::guidance::attach_guidance(&mut messages, &block);
        }

        let temperature = self
            .preset
            .aggregator_temperature
            .unwrap_or(request.temperature);

        Ok(CompletionRequest {
            model: self.aggregator_slot.model.clone(),
            messages: Arc::new(messages),
            tools: Arc::clone(&request.tools),
            max_tokens: request.max_tokens,
            temperature,
            system: request.system.clone(),
            thinking: request.thinking.clone(),
            prompt_caching: request.prompt_caching,
            ..Default::default()
        })
    }

    /// Run (or reuse) the advisor fan-out per the preset cadence.
    ///
    /// Returns the advisor outputs plus whether they came from a fresh
    /// fan-out (cache MISS). Only a MISS may deposit usage or write a trace.
    async fn maybe_run_fanout(
        &self,
        advisors: &[(MoaSlot, Arc<dyn LlmDriver>)],
        advisory_view: &[Message],
    ) -> Fanout {
        let user_prefix = user_turn_prefix_hash(advisory_view);
        let full_sig = signature(advisory_view);

        let should_run = self.update_cadence(user_prefix);

        if !should_run {
            // Skip iteration: reuse the most recent fan-out if we have one.
            if let Some(cached) = self.last_fanout.lock().clone() {
                return Fanout {
                    results: cached.results,
                    fresh: false,
                };
            }
            // Nothing cached yet — fall through and run.
        } else {
            // Cadence says run; but a stable-signature cache HIT still reuses.
            let cache_key = self.cache_key(full_sig, user_prefix);
            if let Some(cached) = self.cache.lock().get(&cache_key).cloned() {
                return Fanout {
                    results: cached.results,
                    fresh: false,
                };
            }
        }

        let results = self.run_fanout_with_events(advisors, advisory_view).await;

        // Cache WRITE only for a completed fan-out.
        let cache_key = self.cache_key(full_sig, user_prefix);
        let cached = CachedFanout {
            results: results.clone(),
        };
        self.cache.lock().insert(cache_key, cached.clone());
        *self.last_fanout.lock() = Some(cached);

        Fanout {
            results,
            fresh: true,
        }
    }

    /// Update the cadence counter and decide whether advisors run this call.
    fn update_cadence(&self, user_prefix: u64) -> bool {
        match self.preset.fanout {
            MoaFanout::UserTurn | MoaFanout::Always => true,
            MoaFanout::EveryN { n } => {
                let n = n.max(1);
                let mut state = self.cadence.lock();
                let counter = if state.last_user_prefix == Some(user_prefix) {
                    state.counter.saturating_add(1)
                } else {
                    1
                };
                state.last_user_prefix = Some(user_prefix);
                state.counter = counter;
                counter == 1 || counter % n == 0
            }
        }
    }

    /// Compute the cache key for the active cadence.
    fn cache_key(&self, full_sig: u64, user_prefix: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.preset_name.hash(&mut hasher);
        match self.preset.fanout {
            MoaFanout::UserTurn => user_prefix.hash(&mut hasher),
            MoaFanout::Always | MoaFanout::EveryN { .. } => full_sig.hash(&mut hasher),
        }
        hasher.finish()
    }

    /// Run the fan-out, emitting progress events.
    ///
    /// `run_fanout` emits a `Progress` event as each advisor lands (live
    /// fan-out progress); after the fan-out settles we emit one `Reference`
    /// per advisor — including `[failed:]`/`[skipped:]` sentinels — then
    /// `Aggregating` when at least one reference output exists.
    async fn run_fanout_with_events(
        &self,
        advisors: &[(MoaSlot, Arc<dyn LlmDriver>)],
        advisory_view: &[Message],
    ) -> Vec<AdvisorResult> {
        let total = advisors.len();
        self.emit(MoaProgressEvent::FanoutStart { total });

        let temperature = effective_reference_temperature(self.preset.reference_temperature);
        let timeout = effective_reference_timeout(self.preset.reference_timeout_secs);

        let results = run_fanout(
            advisors,
            advisory_view,
            temperature,
            timeout,
            self.preset.reference_max_tokens,
            self.catalog.as_ref(),
            self.progress_tx.as_ref(),
        )
        .await;

        let count = results.len();
        for (index, result) in results.iter().enumerate() {
            self.emit(MoaProgressEvent::Reference {
                index,
                count,
                label: result.label.clone(),
            });
        }

        let ref_count = results.iter().filter(|r| r.is_success()).count();
        if ref_count > 0 {
            self.emit(MoaProgressEvent::Aggregating {
                aggregator: slot_label(&self.aggregator_slot),
                ref_count,
            });
        }

        results
    }

    /// Persist a trace record if `save_traces` is enabled.
    ///
    /// A trace is a persisted, user-visible surface, so ANY active privacy
    /// mode (`display` or `full`) redacts the advisor text written here — the
    /// cache itself keeps the raw text so a mid-session mode change neither
    /// leaks nor double-redacts.
    fn maybe_persist_trace(&self, results: &[AdvisorResult]) {
        let moa = &self.config.moa;
        if !moa.save_traces {
            return;
        }
        let redact = moa.privacy_filter.redacts_display();
        let trace_dir = crate::moa::trace::resolve_trace_dir(moa.trace_dir.as_deref());
        let session_id = format!("moa-{}", self.preset_name);
        let record = crate::moa::trace::MoaTraceRecord {
            ts: chrono::Utc::now().to_rfc3339(),
            session_id: session_id.clone(),
            preset: self.preset_name.clone(),
            references: results
                .iter()
                .map(|r| {
                    let mut trace = crate::moa::trace::AdvisorTrace::from_result(r);
                    if redact {
                        trace.output = redact_pii(&trace.output);
                    }
                    trace
                })
                .collect(),
            aggregator: crate::moa::trace::AggregatorTrace {
                label: slot_label(&self.aggregator_slot),
                model: self.aggregator_slot.model.clone(),
                provider: self.aggregator_slot.provider.clone(),
                temperature: self.preset.aggregator_temperature.unwrap_or(0.0),
                input_messages: 0,
                output: String::new(),
                streamed: false,
                output_location: "response".to_string(),
            },
        };
        crate::moa::trace::persist_trace(trace_dir, &session_id, record);
    }

    /// Stamp the aggregator's real provider/model onto a response so billing
    /// attributes spend to the aggregator slot, not the `moa` virtual one.
    fn decorate_response(&self, mut response: CompletionResponse) -> CompletionResponse {
        if response.actual_provider.is_none() {
            response.actual_provider = Some(self.aggregator_slot.provider.clone());
        }
        if response.actual_model.is_none() {
            response.actual_model = Some(self.aggregator_slot.model.clone());
        }
        response
    }
}

#[async_trait]
impl LlmDriver for MoaDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let agg_request = self.prepare_aggregator_request(&request).await?;
        let response = self.aggregator_driver.complete(agg_request).await?;
        Ok(self.decorate_response(response))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let agg_request = self.prepare_aggregator_request(&request).await?;
        let response = self.aggregator_driver.stream(agg_request, tx).await?;
        Ok(self.decorate_response(response))
    }

    fn is_configured(&self) -> bool {
        self.aggregator_driver.is_configured()
    }

    fn family(&self) -> librefang_llm_driver::LlmFamily {
        self.aggregator_driver.family()
    }

    fn is_coding_agent(&self) -> bool {
        self.aggregator_driver.is_coding_agent()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Sum token usage across advisor results.
fn sum_usage(results: &[AdvisorResult]) -> TokenUsage {
    let mut acc = TokenUsage::default();
    for r in results {
        acc.input_tokens += r.usage.input_tokens;
        acc.output_tokens += r.usage.output_tokens;
        acc.cache_creation_input_tokens += r.usage.cache_creation_input_tokens;
        acc.cache_read_input_tokens += r.usage.cache_read_input_tokens;
    }
    acc
}

/// Fold `delta` into `acc` in place.
fn add_usage(acc: &mut TokenUsage, delta: &TokenUsage) {
    acc.input_tokens += delta.input_tokens;
    acc.output_tokens += delta.output_tokens;
    acc.cache_creation_input_tokens += delta.cache_creation_input_tokens;
    acc.cache_read_input_tokens += delta.cache_read_input_tokens;
}

/// Hash the advisory view up to and including the last REAL user message
/// (excluding the synthetic end-on-user marker). Stable across loop
/// iterations within a single user turn.
fn user_turn_prefix_hash(view: &[Message]) -> u64 {
    let last_real = view
        .iter()
        .rposition(|m| m.role == Role::User && m.content.text_content() != END_ON_USER_MARKER);

    let mut hasher = DefaultHasher::new();
    let end = last_real.map(|i| i + 1).unwrap_or(0);
    for msg in &view[..end] {
        msg.role.hash(&mut hasher);
        msg.content.text_content().hash(&mut hasher);
    }
    hasher.finish()
}

/// Hash the entire advisory view. Grows each iteration as assistant turns
/// accumulate, so it changes every loop step.
fn signature(view: &[Message]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for msg in view {
        msg.role.hash(&mut hasher);
        msg.content.text_content().hash(&mut hasher);
    }
    hasher.finish()
}
