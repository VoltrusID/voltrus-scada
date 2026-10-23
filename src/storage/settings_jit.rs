use super::Storage;
use anyhow::Result;

use crate::models::User;

impl Storage {
    // ── JIT User Provisioning (SSO) ──

    /// JIT (Just-in-Time) provisions a user with a default "viewer" role.
    /// Creates the user if they don't already exist. Returns the user ID.
    pub async fn jit_provision_user(
        &self,
        username: &str,
        display_name: &str,
        email: &str,
        role: &str,
    ) -> Result<i64> {
        // Check if user already exists
        if let Some(existing) = self.get_user_by_username(username).await? {
            return Ok(existing.id);
        }

        // Create user with a random unusable password (SSO-authenticated)
        let random_password = format!("sso:{}", hex::encode(rand::random::<[u8; 32]>()));
        let password_hash = crate::api::auth::hash_password(&random_password);
        let id = self.create_user(username, &password_hash, "", role).await?;

        tracing::info!(
            "JIT provisioned user: username={}, role={}, email={}",
            username,
            role,
            email
        );

        // Store external auth metadata in settings KV
        let user_meta = serde_json::json!({
            "display_name": display_name,
            "email": email,
            "provisioned_via": "jit",
            "provisioned_at": crate::timeutil::now_secs(),
        });
        self.upsert_setting(&format!("user_meta:{}", username), &user_meta.to_string())
            .await?;

        Ok(id)
    }

    /// Gets or creates a user by their external identity (provider + external_id).
    /// If the user doesn't exist, JIT provisions them.
    /// Stores the provider/external_id mapping in the settings KV table.
    pub async fn get_or_create_user_by_external_id(
        &self,
        provider: &str,
        external_id: &str,
        default_role: &str,
    ) -> Result<User> {
        let mapping_key = format!("ext_auth:{}:{}", provider, external_id);

        // Check if we have an existing mapping
        let settings = self.get_settings(&[&mapping_key]).await?;
        if let Some(setting) = settings.into_iter().next() {
            // Found existing mapping — look up the user
            if let Some(user) = self.get_user_by_username(&setting.value).await? {
                return Ok(user);
            }
        }

        // No existing mapping — create a new user
        // Derive a username from the external_id
        let username = format!("{}:{}", provider, external_id);
        let user_id = self
            .jit_provision_user(&username, external_id, external_id, default_role)
            .await?;

        // Store the mapping
        self.upsert_setting(&mapping_key, &username).await?;

        // Retrieve and return the newly created user
        self.get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed to retrieve JIT-provisioned user"))
    }
}
