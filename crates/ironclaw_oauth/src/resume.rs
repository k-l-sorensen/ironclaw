use ironclaw_host_api::ResourceScope;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackOutcome {
    pub flow_id: Uuid,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSignal {
    pub credential_name: String,
    pub scope: ResourceScope,
    pub outcome: OAuthCallbackOutcome,
}

#[derive(Debug, Clone)]
pub struct OAuthResumeNotifier {
    sender: broadcast::Sender<ResumeSignal>,
}

impl OAuthResumeNotifier {
    pub fn new(sender: broadcast::Sender<ResumeSignal>) -> Self {
        Self { sender }
    }

    pub fn channel(capacity: usize) -> (Self, broadcast::Receiver<ResumeSignal>) {
        let (sender, receiver) = broadcast::channel(capacity);
        (Self::new(sender), receiver)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ResumeSignal> {
        self.sender.subscribe()
    }

    pub fn notify(&self, credential_name: impl Into<String>, scope: ResourceScope, flow_id: Uuid) {
        self.notify_signal(ResumeSignal {
            credential_name: credential_name.into(),
            scope,
            outcome: OAuthCallbackOutcome {
                flow_id,
                success: true,
            },
        });
    }

    pub fn notify_signal(&self, signal: ResumeSignal) {
        if self.sender.receiver_count() == 0 {
            return;
        }
        let _ = self.sender.send(signal);
    }
}

impl Default for OAuthResumeNotifier {
    fn default() -> Self {
        Self::channel(16).0
    }
}
