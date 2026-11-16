/// An electronic signature sent with an action the edition wants signed:
/// the signed-in user's password, the meaning they were shown and their
/// reason. Never serialized and redacted in `Debug`, so it cannot end up in a
/// log line or an audit entry.
#[derive(Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct SignatureInput {
    pub password: String,
    /// The meaning from the server's `signature_required` answer, unchanged.
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub reason: String,
}

impl std::fmt::Debug for SignatureInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignatureInput")
            .field("password", &"<redacted>")
            .field("meaning", &self.meaning)
            .field("reason", &self.reason)
            .finish()
    }
}

/// The optional body of an action route that had none (acknowledge, recipe
/// load, flow start/stop, go-live): only a signature.
#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct SignedBody {
    #[serde(default)]
    pub signature: Option<SignatureInput>,
}
