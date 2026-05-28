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
        ],
    )
}
