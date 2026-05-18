# `/api/comms/send` does not validate ownership of `from_agent_id` → cross-user impersonation

**Severity:** High
**Category:** Input validation
**Labels:** `security`, `auth`, `impersonation`, `high`

## Affected files
- `crates/librefang-api/src/routes/network.rs:1991-2019` (`comms_send`)

## Description

The handler trusts the literal value of `req.from_agent_id` — it only checks that the `AgentId` exists locally; it **does not verify that the caller owns it**.

Unlike inbound Slack (`channels/slack.rs`), Viber (`channels/viber.rs:21` HMAC-SHA256), and Messenger (`channels/messenger.rs:21` HMAC-SHA1 `X-Hub-Signature`), which do signature validation, `comms_send` is an authenticated route, but the auth layer only proves "some user is logged in," not that the user **owns** `from_agent_id`.

Consequence: a low-privilege user can POST `from_agent_id = <some admin-owned agent>` and forge inter-agent messages from that agent.

## Recommendation

```rust
let api_user = AuthenticatedApiUser::extract(...)?;
let owner = state.kernel.agent_owner(&req.from_agent_id)?;
if owner != api_user.user_id && api_user.role < UserRole::Admin {
    return Err(ApiError::Forbidden);
}
```

Add an integration test in `network_routes_integration.rs`: user A cannot send messages via user B's agent.
