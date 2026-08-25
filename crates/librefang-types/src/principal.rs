//! [`Principal`] — the identity an agent turn acts *for*, and the owner stamped on what that turn creates.
//!
//! # Why a principal rather than a user id
//!
//! An agent is an executor, not an owner.
//! When a support agent builds a workflow during a shift, the workflow belongs to the support rota, not to the agent process that assembled it and not to whichever human happened to be typing at 03:00.
//! Ownership in a real deployment therefore has to be able to name a team, and a team outlives every individual in it (#7744).
//!
//! # Two arms, not three
//!
//! `Principal` is `User | Group`.
//! The original proposal on #7744 had a third `Role(name)` arm, and it was dropped in the issue thread for a reason worth restating where the type lives.
//!
//! [`crate::config::UserConfig::role`] — and the `UserRole` ladder the kernel resolves it into — is an *ordinal privilege level*, `viewer < user < admin < owner`.
//! "Owned by admin" therefore means "owned by everyone at or above admin", which is a permission predicate rather than an identity, and it has no defined answer to the question ownership must answer on deletion: you cannot delete `admin`, so nothing can cascade.
//! The motivating example in the issue — a compliance cron owned by whoever holds that duty this quarter — is a group with rotating membership, and that is exactly what `[[groups]]` (#7745) provides.
//! An identity provider's role claim becomes a LibreFang group under an operator-declared mapping (#7746), so role-shaped ownership is expressible without a role-shaped arm.
//!
//! Note that the group-flavoured *role strings* [`crate::config::KernelConfig::roles_for_user`] returns are a third, unrelated vocabulary — channel binding-rule labels — and are deliberately not principals either.
//!
//! # Why a tagged struct rather than a Rust enum
//!
//! [`Principal`] is `{ kind, id }` with a flat two-field wire shape, not `enum Principal { User(Uuid), Group(Uuid) }`.
//!
//! Every artifact that will carry an owner is `Serialize` + `Deserialize` and reaches `openapi.json` and the four generated SDKs.
//! A Rust enum's serde representation is a *choice* that has to be re-made — and re-published — the day an `Agent` or `Service` arm is added: externally tagged becomes `{"User": "..."}`, adjacently tagged needs `#[serde(tag, content)]` pinned by hand, and either way the shape is an artifact of how many arms exist today.
//! A struct with an explicit `kind` string has one shape forever; a new kind is a new enumerant in an existing field, which is an additive schema change every generated client already tolerates.
//! The cost is that a `match` over `kind` is not exhaustiveness-checked at the call sites that care — accepted, because there are few of them and because the wire shape is the thing that is expensive to get wrong.
//!
//! # Why UUIDv5 rather than the name
//!
//! [`Principal::id`] is derived from the principal's name with UUIDv5, mirroring [`UserId::from_name`] and using a per-kind namespace so `user:alice` and `group:alice` can never collide.
//!
//! This is what makes the owner *queryable without a reverse index*: the derivation is a pure function, so a "show me what the on-call group owns" filter is built by deriving the principal from the name and comparing ids — no join, no lookup table, and the same value on every node.
//! The reverse direction (id back to a display name) is a scan of `[[users]]` / `[[groups]]`, which [`crate::config::KernelConfig::principal_name`] performs; it is cheap because both lists are operator-authored and small.
//!
//! Renaming a principal produces a new id and therefore detaches it from what it used to own.
//! That is the same trade [`UserId::from_name`] already makes, documented there as "rename = new identity", and it is deliberate: an id that tracked renames would need a mutable indirection table, which is the thing UUIDv5 exists to avoid.
//!
//! # Two string forms, deliberately not interchangeable
//!
//! - [`Display`] / [`std::str::FromStr`] are the **canonical storage form**, `kind:uuid` (`user:6f9619ff-8b86-d011-b42d-00c04fc964ff`).
//!   One `TEXT` column holds a whole principal, `WHERE owner = ?` filters on it, and it round-trips exactly.
//! - [`Principal::from_spec`] is the **operator-authored form**, `user:alice` / `group:oncall`, for `config.toml` and `agent.toml`.
//!   The part after the colon is *always* treated as a name and hashed, never as a uuid, because every other identity reference in `config.toml` (`[[users]] name`, `[[groups]] name`, `channel_bindings`) names a principal by name.
//!   Accepting both would make `user:<something uuid-shaped>` ambiguous, and the ambiguity would be silent.

use crate::agent::{UserId, LIBREFANG_USER_NAMESPACE};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable namespace for deriving deterministic group ids from a
/// `[[groups]] name`. Generated once and frozen for the same reason
/// [`LIBREFANG_USER_NAMESPACE`] is: changing it rotates every recorded
/// group owner and silently orphans everything a group owns.
///
/// Distinct from the user namespace so a user and a group that share a
/// name are different principals — the property that lets a single
/// `TEXT` column hold either without a disambiguating join.
pub const LIBREFANG_GROUP_NAMESPACE: Uuid =
    Uuid::from_u128(0x4c46_4147_5f47_524f_5550_5f4e_535f_3501);

/// Which kind of identity a [`Principal`] names.
///
/// Serialized as a lowercase string (`"user"` / `"group"`) so adding a
/// kind is an additive change to the schema rather than a new wire shape.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    /// A single human, identified by [`UserId`].
    User,
    /// A named group from `[[groups]]` in `config.toml` (#7745).
    Group,
}

impl PrincipalKind {
    /// The lowercase wire token for this kind — the same string serde emits.
    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalKind::User => "user",
            PrincipalKind::Group => "group",
        }
    }

    /// The UUIDv5 namespace ids of this kind are derived in.
    pub fn namespace(&self) -> Uuid {
        match self {
            PrincipalKind::User => LIBREFANG_USER_NAMESPACE,
            PrincipalKind::Group => LIBREFANG_GROUP_NAMESPACE,
        }
    }
}

impl std::fmt::Display for PrincipalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The identity an agent turn acts for — a user or a group.
///
/// See the [module documentation](self) for why there are two arms, why this
/// is a tagged struct rather than an enum, and why the id is a UUIDv5.
///
/// `Ord` is derived so a `BTreeSet<Principal>` orders deterministically:
/// anything that reaches an LLM prompt has to be ordered at the boundary
/// (#3298), and an owner list is one feature away from being one.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct Principal {
    /// Whether `id` names a user or a group.
    pub kind: PrincipalKind,
    /// UUIDv5 of the principal's name in [`PrincipalKind::namespace`].
    pub id: Uuid,
}

impl Principal {
    /// A principal naming an already-resolved [`UserId`].
    ///
    /// This is the constructor the turn path uses: the authenticated caller
    /// arrives as a `UserId` and needs no re-derivation.
    pub fn user(id: UserId) -> Self {
        Self {
            kind: PrincipalKind::User,
            id: id.0,
        }
    }

    /// A principal naming the user called `name`, deriving the id the same
    /// way [`UserId::from_name`] does.
    pub fn user_named(name: &str) -> Self {
        Self::user(UserId::from_name(name))
    }

    /// A principal naming the `[[groups]]` entry called `name`.
    pub fn group_named(name: &str) -> Self {
        Self {
            kind: PrincipalKind::Group,
            id: Uuid::new_v5(&LIBREFANG_GROUP_NAMESPACE, name.as_bytes()),
        }
    }

    /// The [`UserId`] this principal names, or `None` when it names a group.
    ///
    /// Lets a call site that genuinely can only act on a single human — per-user
    /// provider credentials, per-user spend budgets — narrow without inventing a
    /// meaning for the group arm.
    pub fn as_user_id(&self) -> Option<UserId> {
        match self.kind {
            PrincipalKind::User => Some(UserId(self.id)),
            PrincipalKind::Group => None,
        }
    }

    /// True when this principal names a group.
    pub fn is_group(&self) -> bool {
        matches!(self.kind, PrincipalKind::Group)
    }

    /// Parse an operator-authored principal spec: `user:alice`, `group:oncall`,
    /// or a bare `alice` (which means `user:alice`, because a bare name in
    /// `config.toml` has always meant a user).
    ///
    /// The value after the colon is **always** a name, never a uuid — see the
    /// [module documentation](self) for why accepting both would be a silent
    /// ambiguity. Use [`str::parse`] for the canonical `kind:uuid` storage form.
    pub fn from_spec(spec: &str) -> Result<Self, PrincipalSpecError> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(PrincipalSpecError::Empty);
        }
        match spec.split_once(':') {
            None => Ok(Self::user_named(spec)),
            Some((kind, name)) => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(PrincipalSpecError::Empty);
                }
                match kind.trim() {
                    "user" => Ok(Self::user_named(name)),
                    "group" => Ok(Self::group_named(name)),
                    other => Err(PrincipalSpecError::UnknownKind(other.to_string())),
                }
            }
        }
    }
}

impl std::fmt::Display for Principal {
    /// The canonical storage form, `kind:uuid`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind.as_str(), self.id)
    }
}

/// Why a canonical `kind:uuid` principal string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalParseError {
    /// No `:` separating the kind from the id.
    Malformed(String),
    /// The kind token is not one this build knows.
    UnknownKind(String),
    /// The id is not a UUID.
    BadUuid(String),
}

impl std::fmt::Display for PrincipalParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrincipalParseError::Malformed(s) => {
                write!(f, "malformed principal `{s}`: expected `kind:uuid`")
            }
            PrincipalParseError::UnknownKind(k) => {
                write!(
                    f,
                    "unknown principal kind `{k}`: expected `user` or `group`"
                )
            }
            PrincipalParseError::BadUuid(s) => write!(f, "principal id `{s}` is not a UUID"),
        }
    }
}

impl std::error::Error for PrincipalParseError {}

impl std::str::FromStr for Principal {
    type Err = PrincipalParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, id) = s
            .split_once(':')
            .ok_or_else(|| PrincipalParseError::Malformed(s.to_string()))?;
        let kind = match kind {
            "user" => PrincipalKind::User,
            "group" => PrincipalKind::Group,
            other => return Err(PrincipalParseError::UnknownKind(other.to_string())),
        };
        let id = Uuid::parse_str(id).map_err(|_| PrincipalParseError::BadUuid(id.to_string()))?;
        Ok(Self { kind, id })
    }
}

/// Why an operator-authored principal spec could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalSpecError {
    /// The spec, or the name half of it, was blank.
    Empty,
    /// The kind token is not one this build knows.
    UnknownKind(String),
}

impl std::fmt::Display for PrincipalSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrincipalSpecError::Empty => f.write_str("principal spec is empty"),
            PrincipalSpecError::UnknownKind(k) => write!(
                f,
                "unknown principal kind `{k}`: expected `user:<name>` or `group:<name>`"
            ),
        }
    }
}

impl std::error::Error for PrincipalSpecError {}

/// Resolve the principal a turn is acting for, in the one place the decision is made (#7744).
///
/// # Precedence, and why
///
/// 1. **The authenticated caller.** A turn started by a human is acting for that human, so what it creates belongs to them.
///    This is the value the kernel already threads as `owner: Option<UserId>` from `AuthenticatedApiUser` — the same one that selects the caller's provider credential and bills their budget — so ownership, credential choice and spend attribution all read one identity by construction instead of three that can disagree.
/// 2. **The agent manifest's `owner`.** Cron fires, event triggers, workflow steps and autonomous ticks have no human on the turn; the kernel hardcodes `owner = None` for every one of them.
///    The manifest is where an operator says who those turns act for, and it is the arm that makes group ownership reachable: `owner = "group:compliance"` on the agent that runs the quarterly cron.
/// 3. **`config.toml: default_owner`.** The fleet-wide fallback, for a deployment that wants every artifact attributed rather than auditing per agent.
/// 4. **`None`.** Stated, not accidental: an artifact created with no resolvable principal is recorded as unowned, and unowned means visible to everyone, because nothing in this increment restricts on ownership.
///
/// The manifest deliberately does not *override* the authenticated caller.
/// An override would mean a support agent configured `owner = "group:support"` silently relabels Alice's workflow as the team's, which is the opposite of "the principal it acted for" and would make the recorded owner unusable as an audit answer.
///
/// A malformed spec at step 2 or 3 logs a `WARN` and falls through to the next step rather than failing the turn: an unparseable owner must not take an agent down.
pub fn resolve_acting_principal(
    authenticated: Option<UserId>,
    manifest_owner: Option<&str>,
    config_default: Option<Principal>,
) -> Option<Principal> {
    if let Some(uid) = authenticated {
        return Some(Principal::user(uid));
    }
    if let Some(spec) = manifest_owner {
        match Principal::from_spec(spec) {
            Ok(p) => return Some(p),
            Err(e) => tracing::warn!(
                manifest_owner = spec,
                error = %e,
                "`owner` in agent.toml is not a valid principal spec (expected `user:<name>` or `group:<name>`); falling back to `default_owner`"
            ),
        }
    }
    config_default
}

/// Emit one `WARN` per daemon naming the config keys that would have given an artifact an owner, then stay quiet.
///
/// Once per process, not once per artifact: a deployment that has declared no principals at all creates unowned artifacts continuously, and a per-artifact warning would be pure noise in exactly the deployment least equipped to act on it.
/// One line is enough to answer "why is nothing owned here" for an operator reading the boot log, and every subsequent occurrence is recorded at `DEBUG` by the call site with the artifact's id, so a specific case is still traceable when someone goes looking.
pub fn warn_once_unowned(artifact_kind: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            first_artifact_kind = artifact_kind,
            "An artifact was created with no owner: the turn had no authenticated caller, the agent's `agent.toml` declares no `owner`, and `config.toml` declares no `default_owner`. Unowned artifacts are visible to everyone and attributable to nobody. Set `default_owner` in config.toml, or `owner` on the agents that run unattended, to attribute them. Logged once per daemon start; subsequent occurrences are at DEBUG."
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_and_group_with_the_same_name_are_different_principals() {
        // The whole reason a single TEXT column can hold either without a
        // disambiguating join. A namespace collision here would silently
        // merge a person and a team.
        let u = Principal::user_named("oncall");
        let g = Principal::group_named("oncall");
        assert_ne!(u.id, g.id);
        assert_ne!(u, g);
        assert_eq!(u.kind, PrincipalKind::User);
        assert_eq!(g.kind, PrincipalKind::Group);
    }

    #[test]
    fn user_principal_id_matches_user_id_from_name() {
        // `Principal::user_named` must agree with the id the rest of the
        // system already derives, or an owner stamped from a name would not
        // match an owner stamped from the authenticated caller.
        assert_eq!(
            Principal::user_named("alice").id,
            UserId::from_name("alice").0
        );
        assert_eq!(
            Principal::user(UserId::from_name("alice")),
            Principal::user_named("alice")
        );
    }

    #[test]
    fn ids_are_stable_across_runs() {
        // UUIDv5 is a pure function of (namespace, name): pinning the literals
        // is what makes an owner recorded today still resolve after a restart,
        // a config reload, or on another node.
        assert_eq!(
            Principal::user_named("alice").to_string(),
            "user:27a0b9d0-692c-5d4b-85f1-281cb49a7ca4"
        );
        assert_eq!(
            Principal::group_named("oncall").to_string(),
            "group:d1aaa362-dded-5767-b881-0d4431418272"
        );
    }

    #[test]
    fn canonical_string_form_round_trips() {
        for p in [
            Principal::user_named("alice"),
            Principal::group_named("support-rota"),
        ] {
            let s = p.to_string();
            let back: Principal = s.parse().expect("canonical form parses");
            assert_eq!(p, back, "`{s}` must round-trip through Display/FromStr");
        }
    }

    #[test]
    fn canonical_parse_rejects_malformed_input() {
        assert!(matches!(
            "no-colon".parse::<Principal>(),
            Err(PrincipalParseError::Malformed(_))
        ));
        assert!(matches!(
            "role:6f9619ff-8b86-d011-b42d-00c04fc964ff".parse::<Principal>(),
            Err(PrincipalParseError::UnknownKind(_))
        ));
        // The decisive one: the canonical form never accepts a name where a
        // uuid belongs, so a spec string cannot be mistaken for storage.
        assert!(matches!(
            "user:alice".parse::<Principal>(),
            Err(PrincipalParseError::BadUuid(_))
        ));
    }

    #[test]
    fn spec_form_always_treats_the_value_as_a_name() {
        assert_eq!(
            Principal::from_spec("user:alice").unwrap(),
            Principal::user_named("alice")
        );
        assert_eq!(
            Principal::from_spec("group:oncall").unwrap(),
            Principal::group_named("oncall")
        );
        // A bare name is a user, matching every other identity reference in
        // `config.toml`.
        assert_eq!(
            Principal::from_spec("alice").unwrap(),
            Principal::user_named("alice")
        );
        // Documented corner: a uuid-shaped *name* is hashed like any other
        // name rather than being adopted as the id. Pinned so the ambiguity
        // stays resolved in one direction on purpose.
        let uuidish = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
        assert_eq!(
            Principal::from_spec(&format!("user:{uuidish}")).unwrap(),
            Principal::user_named(uuidish)
        );
        assert_ne!(
            Principal::from_spec(&format!("user:{uuidish}")).unwrap().id,
            Uuid::parse_str(uuidish).unwrap()
        );
    }

    #[test]
    fn spec_form_rejects_blank_and_unknown_kinds() {
        assert_eq!(Principal::from_spec("   "), Err(PrincipalSpecError::Empty));
        assert_eq!(
            Principal::from_spec("user:"),
            Err(PrincipalSpecError::Empty)
        );
        assert!(matches!(
            Principal::from_spec("role:admin"),
            Err(PrincipalSpecError::UnknownKind(_))
        ));
    }

    #[test]
    fn wire_shape_is_a_flat_tagged_struct() {
        // The shape generated SDKs see. Pinned as a literal because widening
        // `PrincipalKind` must stay an additive change: a Rust enum would have
        // reshaped this object the day a third arm arrived.
        let json = serde_json::to_value(Principal::user_named("alice")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "user",
                "id": "27a0b9d0-692c-5d4b-85f1-281cb49a7ca4",
            })
        );
        let back: Principal = serde_json::from_value(json).unwrap();
        assert_eq!(back, Principal::user_named("alice"));
    }

    #[test]
    fn as_user_id_narrows_only_the_user_arm() {
        assert_eq!(
            Principal::user_named("alice").as_user_id(),
            Some(UserId::from_name("alice"))
        );
        assert_eq!(Principal::group_named("oncall").as_user_id(), None);
        assert!(Principal::group_named("oncall").is_group());
        assert!(!Principal::user_named("alice").is_group());
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    #[test]
    fn the_authenticated_caller_wins_over_every_configured_fallback() {
        // The load-bearing precedence rule: a manifest `owner` must not
        // relabel what a real human asked for.
        let resolved = resolve_acting_principal(
            Some(UserId::from_name("alice")),
            Some("group:support"),
            Some(Principal::group_named("platform")),
        );
        assert_eq!(resolved, Some(Principal::user_named("alice")));
    }

    #[test]
    fn manifest_owner_covers_the_turns_with_no_human_behind_them() {
        // Cron / trigger / workflow-step turns arrive with `authenticated =
        // None`, and this is the arm that makes group ownership reachable.
        let resolved = resolve_acting_principal(
            None,
            Some("group:compliance"),
            Some(Principal::user_named("root")),
        );
        assert_eq!(resolved, Some(Principal::group_named("compliance")));
    }

    #[test]
    fn config_default_is_the_last_resort() {
        let resolved =
            resolve_acting_principal(None, None, Some(Principal::user_named("operator")));
        assert_eq!(resolved, Some(Principal::user_named("operator")));
    }

    #[test]
    fn nothing_configured_resolves_to_unowned() {
        // `None` is the stated answer, not a default nobody chose: with no
        // authenticated caller, no manifest owner and no `default_owner`, an
        // artifact is recorded as unowned rather than attributed to a
        // synthetic principal.
        assert_eq!(resolve_acting_principal(None, None, None), None);
    }

    #[test]
    fn a_malformed_manifest_owner_falls_through_instead_of_failing() {
        let resolved = resolve_acting_principal(
            None,
            Some("role:admin"),
            Some(Principal::user_named("operator")),
        );
        assert_eq!(resolved, Some(Principal::user_named("operator")));
        // …and with nothing to fall through to, unowned rather than a panic.
        assert_eq!(
            resolve_acting_principal(None, Some("role:admin"), None),
            None
        );
    }

    #[test]
    fn a_bare_manifest_owner_name_means_a_user() {
        let resolved = resolve_acting_principal(None, Some("alice"), None);
        assert_eq!(resolved, Some(Principal::user_named("alice")));
    }
}
