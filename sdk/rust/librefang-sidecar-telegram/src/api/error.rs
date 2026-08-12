//! Adapter-wide error type.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Http(reqwest::Error),
    Api {
        method: String,
        code: i32,
        description: String,
    },
    Decode(serde_json::Error),
    Io(std::io::Error),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(e) => write!(f, "http: {e}"),
            Error::Api {
                method,
                code,
                description,
            } => write!(f, "telegram {method}: {code} {description}"),
            Error::Decode(s) => write!(f, "decode: {s}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http(e) => Some(e),
            Error::Decode(e) => Some(e),
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // Strip the URL — reqwest's Display includes it verbatim, and our URLs embed the bot token in the path (`/bot<TOKEN>/method`). Leaking it through logs / protocol error events would compromise the bot.
        Error::Http(e.without_url())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Decode(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_decode_error_remains_in_the_source_chain() {
        let decode_error = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("fixture must be invalid JSON");
        let expected_line = decode_error.line();
        let expected_column = decode_error.column();
        let error = Error::from(decode_error);

        let source = std::error::Error::source(&error).expect("typed decode source");
        let source = source
            .downcast_ref::<serde_json::Error>()
            .expect("source must remain serde_json::Error");
        assert_eq!(source.line(), expected_line);
        assert_eq!(source.column(), expected_column);
    }
}
