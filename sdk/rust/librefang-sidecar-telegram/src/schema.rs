//! Schema declaration emitted by `--describe`.

use librefang_sidecar::{Field, FieldType, Schema};

pub fn telegram_schema() -> Schema {
    Schema::new(
        "telegram",
        "Telegram",
        "Telegram Bot API adapter (out-of-process sidecar).",
        vec![
            Field::new("TELEGRAM_BOT_TOKEN", "Bot Token", FieldType::Secret)
                .required()
                .placeholder("123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"),
            Field::new("ALLOWED_USERS", "Allowed User IDs", FieldType::List)
                .placeholder("123456789, 987654321")
                .advanced(),
            Field::new(
                "TELEGRAM_CLEAR_DONE_REACTION",
                "Clear done reaction",
                FieldType::Bool,
            )
            .advanced(),
            Field::new("TELEGRAM_STREAMING", "Streaming", FieldType::Bool).advanced(),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pins that `telegram_schema()` declares the fields the dashboard
    // `--describe` configure form renders — including TELEGRAM_STREAMING, which
    // the docs advertise as a configurable field (see
    // docs/src/app/architecture/rust-telegram-sidecar/page.mdx); if it were
    // dropped here the toggle would silently never show. This is a schema-side
    // presence check, not a docs parser, so the two must be kept in sync by
    // hand. Lives inline because this is a binary-only crate and
    // `telegram_schema` is not exported to a lib target.
    #[test]
    fn schema_declares_expected_fields() {
        let schema = telegram_schema();
        assert_eq!(schema.name, "telegram");

        let bot_token = schema
            .fields
            .iter()
            .find(|f| f.key == "TELEGRAM_BOT_TOKEN")
            .expect("schema must declare TELEGRAM_BOT_TOKEN");
        assert_eq!(bot_token.field_type, FieldType::Secret);
        assert!(bot_token.required);

        let streaming = schema
            .fields
            .iter()
            .find(|f| f.key == "TELEGRAM_STREAMING")
            .expect("schema must declare TELEGRAM_STREAMING");
        assert_eq!(streaming.field_type, FieldType::Bool);
        assert!(streaming.advanced);
    }
}
