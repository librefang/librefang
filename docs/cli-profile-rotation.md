# Claude Code CLI Profile Rotation

Automatically rotate between multiple Claude Code accounts when one hits its rate limit. LibreFang tries the next account transparently — no downtime, no manual intervention.

## Why

Claude Code CLI accounts have per-user rate limits. When you hit "You've hit your limit · resets 10am (UTC)", you're stuck. With profile rotation, LibreFang switches to the next available account and keeps going.

## How it works

1. Each Claude Code account has its own config directory with separate OAuth credentials
2. LibreFang creates one `ClaudeCodeDriver` per account, wrapped in `TokenRotationDriver`
3. On rate-limit, auth failure, or OAuth expiry, the current account enters cooldown and the next one takes over
4. When all accounts are exhausted, the error with the earliest reset time is returned

```
User → LibreFang → TokenRotationDriver
                    ├── ClaudeCodeDriver (account 1) ← active
                    ├── ClaudeCodeDriver (account 2) ← standby
                    └── ClaudeCodeDriver (account 3) ← standby
                    
Rate limit on account 1 → auto-switch to account 2
```

## Setup

### Step 1: Create additional Claude accounts

Each account needs its own `claude` CLI login:

```bash
# Account 2
mkdir -p ~/.claude-profiles/account-2
CLAUDE_CONFIG_DIR=~/.claude-profiles/account-2 claude auth login

# Account 3
mkdir -p ~/.claude-profiles/account-3
CLAUDE_CONFIG_DIR=~/.claude-profiles/account-3 claude auth login
```

Each directory will contain `.credentials.json` and session data after login.

### Step 2: Configure LibreFang

Add the profile directories to your `config.toml`:

```toml
[default_model]
provider = "claude_code"
model = "claude-sonnet-4-20250514"
cli_profile_dirs = [
    "~/.claude",                          # primary account
    "~/.claude-profiles/account-2",       # secondary
    "~/.claude-profiles/account-3",       # tertiary
]
```

### Step 3: Restart the daemon

```bash
librefang start --foreground
```

You should see in the logs:

```
INFO Claude Code CLI profile rotation enabled pool_size=3
```

## What triggers rotation

| Error | Rotates? | Why |
|-------|----------|-----|
| Rate limited (429) | Yes | Account quota exhausted |
| "You've hit your limit" in CLI output | Yes | Claude Code specific rate limit text |
| OAuth token expired (401) | Yes | Token on this account expired, next may be valid |
| "not authenticated" in CLI output | Yes | CLI auth issue, try another profile |
| Overloaded (529) | Yes | Server overload, spread across accounts |
| Billing error (402) | Yes | Account billing issue |
| Invalid API key (403) | No | Permanent — rotating won't help |
| Parse/network errors | No | Not account-specific |

## Cooldown behavior

When an account is rate-limited:
- It enters cooldown for the duration specified by the provider (minimum 30 seconds)
- If the error message includes a reset time ("resets 10am UTC"), the cooldown is calculated to that exact hour
- During cooldown, that account is skipped in the rotation
- When cooldown expires, the account rejoins the pool

## Verifying it works

Check the daemon logs for rotation events:

```bash
grep -iE 'rotation|rotating|profile|exhausted' /path/to/librefang.log
```

You should see:
```
INFO Claude Code CLI profile rotation enabled pool_size=3
WARN Rate limited on profile-1, rotating to profile-2
WARN All profiles exhausted, earliest reset: 10:00 UTC
```

## FAQ

**Q: Do I need different email accounts?**  
A: Yes, each profile needs a separate Claude account with its own subscription.

**Q: Can I mix Claude Code CLI with API key accounts?**  
A: The existing `auth_profiles` config handles API key rotation separately. `cli_profile_dirs` is specifically for Claude Code CLI accounts. Both can coexist.

**Q: What if all accounts are rate-limited?**  
A: LibreFang returns the error from the account that resets soonest, so the user/agent knows when to retry.

**Q: Does this work with Claude Max/Pro plans?**  
A: Yes, it works with any Claude plan that has a rate limit.
