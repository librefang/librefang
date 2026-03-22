//! 自定义序列化/反序列化辅助函数。

use serde::{Deserialize, Serialize};

/// Deserialize a `Vec<String>` that tolerates both string and integer elements.
///
/// When channel configs are saved from the web dashboard, numeric IDs (e.g. Discord
/// guild snowflakes, Telegram user IDs) are stored as TOML integers. This helper
/// transparently converts integers back to strings so deserialization never fails.
pub(crate) fn deserialize_string_or_int_vec<'de, D>(
    deserializer: D,
) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values: Vec<serde_json::Value> = serde::Deserialize::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        })
        .collect())
}

/// Config field that accepts either a single value or an array.
/// Enables multi-bot configurations while staying backward-compatible.
///
/// TOML single-instance: `[channels.telegram]`
/// TOML multi-instance:  `[[channels.telegram]]`
#[derive(Debug, Clone)]
pub struct OneOrMany<T>(pub Vec<T>);

impl<T> OneOrMany<T> {
    /// Returns true if no values are present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Returns the number of values.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Returns a reference to the first value, if any.
    pub fn first(&self) -> Option<&T> {
        self.0.first()
    }
    /// Returns an iterator over the values.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }
    /// Backward-compat: replaces `Option::is_some()`.
    pub fn is_some(&self) -> bool {
        !self.0.is_empty()
    }
    /// Backward-compat: replaces `Option::is_none()`.
    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }
    /// Backward-compat: replaces `Option::as_ref()` — returns the first value.
    pub fn as_ref(&self) -> Option<&T> {
        self.0.first()
    }
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Serialize> Serialize for OneOrMany<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.len() {
            0 => serializer.serialize_none(),
            1 => self.0[0].serialize(serializer),
            _ => self.0.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct OneOrManyVisitor<T>(std::marker::PhantomData<T>);

        impl<'de, T: Deserialize<'de>> de::Visitor<'de> for OneOrManyVisitor<T> {
            type Value = OneOrMany<T>;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a single value or array of values")
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(val) = seq.next_element()? {
                    v.push(val);
                }
                Ok(OneOrMany(v))
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let val = T::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(OneOrMany(vec![val]))
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(OneOrMany(Vec::new()))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(OneOrMany(Vec::new()))
            }
        }

        deserializer.deserialize_any(OneOrManyVisitor(std::marker::PhantomData))
    }
}
