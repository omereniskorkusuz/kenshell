//! Managed application state.

use std::sync::Arc;

use crate::console_detect::ConsoleRegistry;
use crate::handoff::Handoffs;
use crate::secrets::SecretStore;
use crate::session::SessionManager;
use crate::storage::Db;

pub struct AppState {
    pub db: Arc<Db>,
    pub consoles: Arc<ConsoleRegistry>,
    pub sessions: Arc<SessionManager>,
    pub secrets: Arc<dyn SecretStore>,
    pub handoffs: Handoffs,
    /// sFlow / NetFlow collector for the topology map (idle until switched on).
    pub flow: Arc<crate::flow::FlowCollector>,
}
