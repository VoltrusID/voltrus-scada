//! The phone layout override of one widget (docs/designer.md, phase 5):
//! `config.mobile = {hide?, order?, span?}`. The operator layout on phones is
//! generated from the screen; a widget carries this key only when an engineer
//! adjusted it in the Designer's *Mobile* pane. It rides in the row's
//! `config`, so every save path (layout `PUT`s, the change log, publishing,
//! the collaboration document, project files) carries it like any other key.
//! [`check`] is the shape every writer must respect; the server applies it to
//! REST saves, collaborative updates and project imports.

use serde_json::Value;

/// The key in a widget row's `config`.
pub const KEY: &str = "mobile";

/// Largest `order` (absolute value).
pub const MAX_ORDER: f64 = 1_000_000.0;

/// Whether `v` is a valid override: an object with only `hide` (a boolean),
/// `order` (a finite number within [`MAX_ORDER`]) and `span` (1 or 2), each
/// optional. `null` counts as no override.
pub fn check(v: &Value) -> Result<(), String> {
    let map = match v {
        Value::Null => return Ok(()),
        Value::Object(m) => m,
        _ => return Err("mobile is not an object".into()),
    };
    for (k, val) in map {
        match k.as_str() {
            "hide" => {
                if !val.is_boolean() {
                    return Err("mobile.hide is not true or false".into());
                }
            }
            "order" => {
                if !val
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && n.abs() <= MAX_ORDER)
                {
                    return Err("mobile.order is not a number within ±1000000".into());
                }
            }
            "span" => {
                if val.is_f64() || !matches!(val.as_u64(), Some(1 | 2)) {
                    return Err("mobile.span is not 1 or 2".into());
                }
            }
            other => {
                return Err(format!(
                    "mobile.{} is not a known key (hide, order, span)",
                    other.chars().take(32).collect::<String>()
                ));
            }
        }
    }
    Ok(())
}

/// [`check`] of the override in a widget's `config` object, if any.
pub fn check_config(config: &serde_json::Map<String, Value>) -> Result<(), String> {
    config.get(KEY).map_or(Ok(()), check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_the_three_keys_each_optional() {
        for ok in [
            json!(null),
            json!({}),
            json!({"hide": true}),
            json!({"hide": false}),
            json!({"order": 3}),
            json!({"order": -2.5}),
            json!({"span": 1}),
            json!({"span": 2}),
            json!({"hide": false, "order": 0, "span": 1}),
        ] {
            assert_eq!(check(&ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn refuses_other_shapes() {
        for bad in [
            json!(true),
            json!([1]),
            json!("hide"),
            json!({"hide": 1}),
            json!({"hide": "yes"}),
            json!({"order": "1"}),
            json!({"order": 1e9}),
            json!({"span": 3}),
            json!({"span": 0}),
            json!({"span": 1.5}),
            json!({"span": 2.0}),
            json!({"span": "2"}),
            json!({"colour": "red"}),
        ] {
            assert!(check(&bad).is_err(), "{bad}");
        }
        let mut cfg = serde_json::Map::new();
        assert_eq!(check_config(&cfg), Ok(()));
        cfg.insert(KEY.into(), json!({"span": 9}));
        assert!(check_config(&cfg).unwrap_err().contains("span"));
    }
}
