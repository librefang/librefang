//! Access control parsed from the `ALLOWED_USERS` schema field.
//!
//! Empty allowlist ⇒ permit-all. Numeric entries match user_id exactly; entries starting with `@` (or any alphabetic char) match the user's username case-insensitively with the leading `@` optional.

pub struct AllowList {
    raw_ids: Vec<String>,
    usernames_lower: Vec<String>,
}

impl AllowList {
    pub fn from_env_value(value: Option<&str>) -> Self {
        let mut raw_ids = Vec::new();
        let mut usernames_lower = Vec::new();
        if let Some(s) = value {
            for token in s.split(',') {
                let t = token.trim();
                if t.is_empty() {
                    continue;
                }
                if t.chars().all(|c| c.is_ascii_digit()) {
                    raw_ids.push(t.to_string());
                } else {
                    usernames_lower.push(t.trim_start_matches('@').to_ascii_lowercase());
                }
            }
        }
        Self {
            raw_ids,
            usernames_lower,
        }
    }

    pub fn is_open(&self) -> bool {
        self.raw_ids.is_empty() && self.usernames_lower.is_empty()
    }

    pub fn permits(&self, user_id: &str, username: Option<&str>) -> bool {
        if self.is_open() {
            return true;
        }
        if self.raw_ids.iter().any(|id| id == user_id) {
            return true;
        }
        if let Some(uname) = username {
            let lower = uname.trim_start_matches('@').to_ascii_lowercase();
            if self.usernames_lower.iter().any(|u| u == &lower) {
                return true;
            }
        }
        false
    }
}
