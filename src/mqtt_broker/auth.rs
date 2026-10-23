use crate::models::MqttAuthConfig;

/// Validate MQTT credentials against the configured auth settings.
pub fn validate_credentials(
    auth_config: &MqttAuthConfig,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    if auth_config.mode == "none" || auth_config.users.is_empty() {
        return Ok(());
    }

    let username = username.ok_or_else(|| "Username required when auth is enabled".to_string())?;
    let password = password.ok_or_else(|| "Password required when auth is enabled".to_string())?;

    let user = auth_config
        .users
        .iter()
        .find(|u| u.username == username)
        .ok_or_else(|| format!("Unknown MQTT user: {}", username))?;

    if user.password != password {
        return Err("Invalid MQTT password".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MqttUser;

    #[test]
    fn test_auth_disabled() {
        let config = MqttAuthConfig {
            mode: "none".to_string(),
            users: vec![],
        };
        let result = validate_credentials(&config, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_enabled_valid() {
        let config = MqttAuthConfig {
            mode: "password".to_string(),
            users: vec![MqttUser {
                username: "admin".to_string(),
                password: "secret".to_string(),
            }],
        };
        let result = validate_credentials(&config, Some("admin"), Some("secret"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_enabled_invalid_password() {
        let config = MqttAuthConfig {
            mode: "password".to_string(),
            users: vec![MqttUser {
                username: "admin".to_string(),
                password: "secret".to_string(),
            }],
        };
        let result = validate_credentials(&config, Some("admin"), Some("wrong"));
        assert!(result.is_err());
    }
}
