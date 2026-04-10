//! Integration tests for webhook tenant isolation in multi-tenant environment.

use librefang_api::webhook_store::{
    CreateWebhookRequest, UpdateWebhookRequest, WebhookEvent, WebhookId, WebhookStore,
};
use std::path::PathBuf;
use uuid::Uuid;

fn temp_store() -> (WebhookStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("webhooks.json");
    (WebhookStore::load(path), dir)
}

fn create_webhook_req(name: &str) -> CreateWebhookRequest {
    CreateWebhookRequest {
        name: name.to_string(),
        url: "https://example.com/webhook".to_string(),
        secret: Some("secret".to_string()),
        events: vec![WebhookEvent::AgentSpawned],
        enabled: true,
    }
}

#[test]
fn test_webhook_list_respects_tenant_boundary() {
    let (store, _dir) = temp_store();
    let tenant_a = "org-alpha".to_string();
    let tenant_b = "org-beta".to_string();

    // Tenant A creates 3 webhooks
    store
        .create_scoped(create_webhook_req("hook-a1"), tenant_a.clone())
        .unwrap();
    store
        .create_scoped(create_webhook_req("hook-a2"), tenant_a.clone())
        .unwrap();
    store
        .create_scoped(create_webhook_req("hook-a3"), tenant_a.clone())
        .unwrap();

    // Tenant B creates 2 webhooks
    store
        .create_scoped(create_webhook_req("hook-b1"), tenant_b.clone())
        .unwrap();
    store
        .create_scoped(create_webhook_req("hook-b2"), tenant_b.clone())
        .unwrap();

    // Verify tenant A only sees their 3 webhooks
    let a_webhooks = store.list_scoped(&tenant_a);
    assert_eq!(a_webhooks.len(), 3);
    assert!(a_webhooks.iter().all(|w| w.account_id == tenant_a));

    // Verify tenant B only sees their 2 webhooks
    let b_webhooks = store.list_scoped(&tenant_b);
    assert_eq!(b_webhooks.len(), 2);
    assert!(b_webhooks.iter().all(|w| w.account_id == tenant_b));
}

#[test]
fn test_webhook_get_scoped_isolation() {
    let (store, _dir) = temp_store();
    let tenant_a = "company-x".to_string();
    let tenant_b = "company-y".to_string();

    // Tenant A creates a webhook
    let wh_a = store
        .create_scoped(create_webhook_req("webhook-x"), tenant_a.clone())
        .unwrap();

    // Tenant B creates a webhook
    let wh_b = store
        .create_scoped(create_webhook_req("webhook-y"), tenant_b.clone())
        .unwrap();

    // Tenant A can retrieve their own webhook
    let found_a = store.get_scoped(wh_a.id, &tenant_a);
    assert!(found_a.is_some());
    assert_eq!(found_a.unwrap().id, wh_a.id);

    // Tenant A cannot retrieve tenant B's webhook
    let not_found = store.get_scoped(wh_b.id, &tenant_a);
    assert!(not_found.is_none());

    // Tenant B can retrieve their own webhook
    let found_b = store.get_scoped(wh_b.id, &tenant_b);
    assert!(found_b.is_some());
    assert_eq!(found_b.unwrap().id, wh_b.id);

    // Tenant B cannot retrieve tenant A's webhook
    let not_found = store.get_scoped(wh_a.id, &tenant_b);
    assert!(not_found.is_none());
}

#[test]
fn test_webhook_update_respects_tenant_isolation() {
    let (store, _dir) = temp_store();
    let tenant_a = "startup-1".to_string();
    let tenant_b = "startup-2".to_string();

    let wh_a = store
        .create_scoped(create_webhook_req("startup-hook"), tenant_a.clone())
        .unwrap();

    // Tenant B attempts to update tenant A's webhook
    let result = store.update_scoped(
        wh_a.id,
        &tenant_b,
        UpdateWebhookRequest {
            name: Some("hijacked".to_string()),
            url: None,
            secret: None,
            events: None,
            enabled: None,
        },
    );

    // Update should fail (webhook not found for tenant_b)
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));

    // Verify the original webhook is unchanged
    let unchanged = store.get_scoped(wh_a.id, &tenant_a).unwrap();
    assert_eq!(unchanged.name, "startup-hook");
}

#[test]
fn test_webhook_delete_respects_tenant_isolation() {
    let (store, _dir) = temp_store();
    let tenant_a = "client-a".to_string();
    let tenant_b = "client-b".to_string();

    let wh_a = store
        .create_scoped(create_webhook_req("client-a-hook"), tenant_a.clone())
        .unwrap();
    let wh_b = store
        .create_scoped(create_webhook_req("client-b-hook"), tenant_b.clone())
        .unwrap();

    // Tenant B attempts to delete tenant A's webhook
    let deleted = store.delete_scoped(wh_a.id, &tenant_b);

    // Delete should fail (webhook not found for tenant_b)
    assert!(!deleted);

    // Verify tenant A's webhook still exists
    assert!(store.get_scoped(wh_a.id, &tenant_a).is_some());
    assert_eq!(store.list_scoped(&tenant_a).len(), 1);

    // Tenant A can delete their own webhook
    let deleted = store.delete_scoped(wh_a.id, &tenant_a);
    assert!(deleted);
    assert!(store.get_scoped(wh_a.id, &tenant_a).is_none());

    // Tenant B's webhook is unaffected
    assert!(store.get_scoped(wh_b.id, &tenant_b).is_some());
}

#[test]
fn test_concurrent_tenant_operations_do_not_interfere() {
    let (store, _dir) = temp_store();
    let tenant_1 = "tenant-1".to_string();
    let tenant_2 = "tenant-2".to_string();
    let tenant_3 = "tenant-3".to_string();

    // Each tenant creates webhooks with similar names
    let wh1 = store
        .create_scoped(create_webhook_req("app-hook"), tenant_1.clone())
        .unwrap();
    let wh2 = store
        .create_scoped(create_webhook_req("app-hook"), tenant_2.clone())
        .unwrap();
    let wh3 = store
        .create_scoped(create_webhook_req("app-hook"), tenant_3.clone())
        .unwrap();

    // All webhooks have the same name but different IDs and account_ids
    assert_eq!(wh1.name, wh2.name);
    assert_eq!(wh2.name, wh3.name);
    assert_ne!(wh1.id, wh2.id);
    assert_ne!(wh2.id, wh3.id);
    assert_ne!(wh1.account_id, wh2.account_id);
    assert_ne!(wh2.account_id, wh3.account_id);

    // Each tenant's list contains exactly 1 webhook
    assert_eq!(store.list_scoped(&tenant_1).len(), 1);
    assert_eq!(store.list_scoped(&tenant_2).len(), 1);
    assert_eq!(store.list_scoped(&tenant_3).len(), 1);

    // Tenant 1 updates their webhook
    let updated = store
        .update_scoped(
            wh1.id,
            &tenant_1,
            UpdateWebhookRequest {
                name: Some("updated-hook".to_string()),
                url: None,
                secret: None,
                events: None,
                enabled: Some(false),
            },
        )
        .unwrap();

    assert_eq!(updated.name, "updated-hook");
    assert!(!updated.enabled);

    // Verify other tenants' webhooks are unaffected
    let wh2_unchanged = store.get_scoped(wh2.id, &tenant_2).unwrap();
    assert_eq!(wh2_unchanged.name, "app-hook");
    assert!(wh2_unchanged.enabled);

    let wh3_unchanged = store.get_scoped(wh3.id, &tenant_3).unwrap();
    assert_eq!(wh3_unchanged.name, "app-hook");
    assert!(wh3_unchanged.enabled);
}

#[test]
fn test_webhook_account_id_persisted_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("webhooks.json");
    let tenant_a = "persistent-a".to_string();
    let tenant_b = "persistent-b".to_string();

    // Create and persist webhooks
    {
        let store = WebhookStore::load(path.clone());
        store
            .create_scoped(create_webhook_req("persistent-hook-a"), tenant_a.clone())
            .unwrap();
        store
            .create_scoped(create_webhook_req("persistent-hook-b"), tenant_b.clone())
            .unwrap();
    }

    // Reload and verify account_id is correctly persisted
    {
        let store = WebhookStore::load(path);
        let a_webhooks = store.list_scoped(&tenant_a);
        let b_webhooks = store.list_scoped(&tenant_b);

        assert_eq!(a_webhooks.len(), 1);
        assert_eq!(b_webhooks.len(), 1);

        assert_eq!(a_webhooks[0].account_id, tenant_a);
        assert_eq!(b_webhooks[0].account_id, tenant_b);
    }
}
