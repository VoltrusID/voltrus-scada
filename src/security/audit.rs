#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub action: String,
    pub username: String,
    pub resource: String,
    pub details: Option<String>,
}

#[derive(Clone)]
pub struct AuditLogger {
    sender: tokio::sync::mpsc::UnboundedSender<AuditEvent>,
}

impl AuditLogger {
    pub fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<AuditEvent>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { sender: tx }, rx)
    }

    pub fn log(&self, event: AuditEvent) {
        let _ = self.sender.send(event);
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Self { sender: tx }
    }
}
