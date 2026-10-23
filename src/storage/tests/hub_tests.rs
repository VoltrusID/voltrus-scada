use super::*;

#[tokio::test]
async fn test_hub_site_crud() {
    let storage = Storage::new_in_memory().unwrap();

    // Create a site
    let id = storage
        .create_hub_site(
            "Factory A",
            "https://factory-a.example.com",
            "secret-key-123",
            true,
        )
        .await
        .unwrap();
    assert!(id > 0);

    // List sites
    let sites = storage.list_hub_sites().await.unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].name, "Factory A");
    assert_eq!(sites[0].url, "https://factory-a.example.com");
    assert_eq!(sites[0].enabled, true);
    assert!(sites[0].last_seen.is_none());

    // Update site
    storage
        .update_hub_site(id, Some("Factory A Updated"), None, None, None)
        .await
        .unwrap();
    let sites = storage.list_hub_sites().await.unwrap();
    assert_eq!(sites[0].name, "Factory A Updated");

    // Update last_seen
    storage.update_hub_site_last_seen(id).await.unwrap();
    let sites = storage.list_hub_sites().await.unwrap();
    assert!(sites[0].last_seen.is_some());

    // Get enabled sites
    let enabled = storage.get_enabled_hub_sites().await.unwrap();
    assert_eq!(enabled.len(), 1);

    // Disable site
    storage
        .update_hub_site(id, None, None, None, Some(false))
        .await
        .unwrap();
    let enabled = storage.get_enabled_hub_sites().await.unwrap();
    assert_eq!(enabled.len(), 0);

    // Delete site
    storage.delete_hub_site(id).await.unwrap();
    let sites = storage.list_hub_sites().await.unwrap();
    assert_eq!(sites.len(), 0);
}

#[tokio::test]
async fn test_hub_site_multiple_sites() {
    let storage = Storage::new_in_memory().unwrap();

    let id1 = storage
        .create_hub_site("Site 1", "https://site1.example.com", "key1", true)
        .await
        .unwrap();
    let id2 = storage
        .create_hub_site("Site 2", "https://site2.example.com", "key2", false)
        .await
        .unwrap();

    let all = storage.list_hub_sites().await.unwrap();
    assert_eq!(all.len(), 2);

    let enabled = storage.get_enabled_hub_sites().await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, id1);

    // Delete one
    storage.delete_hub_site(id1).await.unwrap();
    let all = storage.list_hub_sites().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, id2);
}
