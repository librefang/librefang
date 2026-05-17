//! Pre-dispatch provider budget gate.

use arc_swap::ArcSwap;
use librefang_types::config::ProviderBudget;
use librefang_types::error::LibreFangResult;
use std::collections::HashMap;
use std::sync::Arc;

use crate::MeteringEngine;

/// Shared gate consulted before dispatching to an LLM provider.
///
/// Backed by SQLite (via [`MeteringEngine`]) so it reflects cross-agent
/// usage. Budget config is hot-reloadable via [`ArcSwap`].
pub struct ProviderBudgetGate {
    metering: Arc<MeteringEngine>,
    budgets: ArcSwap<HashMap<String, ProviderBudget>>,
}

impl ProviderBudgetGate {
    pub fn new(metering: Arc<MeteringEngine>, budgets: HashMap<String, ProviderBudget>) -> Self {
        Self {
            metering,
            budgets: ArcSwap::new(Arc::new(budgets)),
        }
    }

    /// Check whether the given provider's budget allows another call.
    pub fn check(&self, provider: &str) -> LibreFangResult<()> {
        let budgets = self.budgets.load();
        if let Some(pb) = budgets.get(provider) {
            self.metering.check_provider_budget(provider, pb)?;
        }
        Ok(())
    }

    /// Update budgets on hot-reload.
    pub fn update_budgets(&self, budgets: HashMap<String, ProviderBudget>) {
        self.budgets.store(Arc::new(budgets));
    }
}
