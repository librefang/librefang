//! `librefang group …` — user-group management from the terminal (#7745).
//!
//! Thin client over `/api/groups`.
//! Nothing is decided here that the daemon does not decide: name validation, member de-duplication and sorting, the idempotence of add/remove, and the Owner-only gate all live server-side, so a group edited from the TUI, the dashboard and this command lands identically.
//!
//! Groups are flat — there is no `add-child` verb because groups do not nest.
//! The reasoning is on `GroupConfig` in `librefang-types`.

use crate::commands::prelude::*;

/// Render a group's JSON body as a human-readable block.
fn print_group(body: &serde_json::Value) {
    ui::kv(
        &i18n::t("group-label-name"),
        body["name"].as_str().unwrap_or("?"),
    );
    let description = body["description"].as_str().unwrap_or("");
    if !description.is_empty() {
        ui::kv(&i18n::t("group-label-description"), description);
    }
    let roles = body["roles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if !roles.is_empty() {
        ui::kv(&i18n::t("group-label-roles"), &roles);
    }
    let members = body["members"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    ui::kv(
        &i18n::t("group-label-members"),
        if members.is_empty() { "—" } else { &members },
    );
    // Members with no `[[users]]` entry are legitimate (an identity provider
    // can name someone before their first sign-in), so this is information,
    // not an error — but it is worth surfacing rather than hiding.
    let unknown = body["unknown_members"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if !unknown.is_empty() {
        ui::kv(&i18n::t("group-label-unregistered"), &unknown);
    }
}

fn print_json(body: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(body).unwrap_or_default());
}

pub(crate) fn cmd_group_list(json: bool) {
    let base = require_daemon("group list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/groups")).send());
    if json {
        print_json(&body);
        return;
    }
    let Some(arr) = body.as_array() else {
        print_json(&body);
        return;
    };
    if arr.is_empty() {
        println!("{}", i18n::t("group-none"));
        return;
    }
    let header_name = i18n::t("label-header-name");
    let header_members = i18n::t("group-header-members");
    let header_roles = i18n::t("group-header-roles");
    let header_description = i18n::t("group-header-description");
    let mut t = crate::table::Table::new(&[
        &header_name,
        &header_members,
        &header_roles,
        &header_description,
    ]);
    for g in arr {
        let member_count = g["member_count"].as_u64().unwrap_or(0).to_string();
        let roles = g["roles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        t.add_row(&[
            g["name"].as_str().unwrap_or("?"),
            &member_count,
            &roles,
            g["description"].as_str().unwrap_or(""),
        ]);
    }
    t.print();
}

pub(crate) fn cmd_group_show(name: &str, json: bool) {
    let base = require_daemon("group show");
    let client = daemon_client();
    let (status, body) = daemon_json_checked(
        client
            .get(format!("{base}/api/groups/{}", urlencode(name)))
            .send(),
    );
    if json {
        print_json(&body);
        return;
    }
    if !status.is_success() {
        ui::error(&i18n::t_args(
            "group-show-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        return;
    }
    ui::section(&i18n::t("group-section-detail"));
    print_group(&body);
}

pub(crate) fn cmd_group_create(name: &str, description: Option<&str>, roles: &[String]) {
    let base = require_daemon("group create");
    let client = daemon_client();
    let payload = serde_json::json!({
        "name": name,
        "description": description.unwrap_or(""),
        "roles": roles,
    });
    let (status, body) = daemon_json_checked(
        client
            .post(format!("{base}/api/groups"))
            .json(&payload)
            .send(),
    );
    if !status.is_success() {
        ui::error(&i18n::t_args(
            "group-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        return;
    }
    ui::success(&i18n::t_args("group-created", &[("name", name)]));
    print_group(&body);
}

pub(crate) fn cmd_group_delete(name: &str) {
    let base = require_daemon("group delete");
    let client = daemon_client();
    let (status, body) = daemon_json_checked(
        client
            .delete(format!("{base}/api/groups/{}", urlencode(name)))
            .send(),
    );
    if !status.is_success() {
        ui::error(&i18n::t_args(
            "group-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        return;
    }
    ui::success(&i18n::t_args("group-deleted", &[("name", name)]));
}

pub(crate) fn cmd_group_member(group: &str, user: &str, add: bool) {
    let label = if add {
        "group add-member"
    } else {
        "group remove-member"
    };
    let base = require_daemon(label);
    let client = daemon_client();
    let url = format!(
        "{base}/api/groups/{}/members/{}",
        urlencode(group),
        urlencode(user)
    );
    let request = if add {
        client.put(url).json(&serde_json::json!({}))
    } else {
        client.delete(url)
    };
    let (status, body) = daemon_json_checked(request.send());
    if !status.is_success() {
        ui::error(&i18n::t_args(
            "group-member-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        return;
    }
    let key = if add {
        "group-member-added"
    } else {
        "group-member-removed"
    };
    ui::success(&i18n::t_args(key, &[("user", user), ("group", group)]));
    print_group(&body);
}

/// `librefang group of <user>` — the reverse lookup.
///
/// Answers "which teams is this person on, and what does that confer", which
/// is the question every consumer of the group model actually asks.
pub(crate) fn cmd_group_of(user: &str, json: bool) {
    let base = require_daemon("group of");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/users/{}/groups", urlencode(user)))
            .send(),
    );
    if json {
        print_json(&body);
        return;
    }
    let groups = body["groups"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if groups.is_empty() {
        println!("{}", i18n::t_args("group-user-none", &[("user", user)]));
        return;
    }
    ui::kv(&i18n::t("group-label-groups"), &groups);
    let roles = body["roles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    ui::kv(&i18n::t("group-label-roles"), &roles);
}

/// Percent-encode a path segment.
///
/// Group and user names are operator-chosen and may legitimately contain a
/// space or a slash; pasting one straight into the URL would either produce a
/// malformed request or address a different resource entirely.
fn urlencode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn urlencode_escapes_path_separators_and_spaces() {
        assert_eq!(urlencode("on call"), "on%20call");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("plain-name_1.0~x"), "plain-name_1.0~x");
    }

    #[test]
    fn urlencode_is_utf8_byte_wise() {
        // Multi-byte characters must be escaped per UTF-8 byte, not per char,
        // or the daemon receives a different name than the operator typed.
        assert_eq!(urlencode("é"), "%C3%A9");
    }
}
