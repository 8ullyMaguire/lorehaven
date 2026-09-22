//! Mirror admin & IPFS (spec §32.7.6): local mirrors, IPFS pins, DMCA takedowns.

use lorehaven_db::media_resilience;

#[tokio::test]
async fn local_mirror_insert_and_list() {
    let dir = test_support::scratch_dir("ma_lm");
    let tdb = test_support::TestDb::connect_with_dir("ma-lm", &dir).await;
    let db = tdb.db();

    let mirror_id = "mirror-001";
    let reference_id = "ref-001";

    media_resilience::insert_local_mirror(
        db, mirror_id, reference_id,
        "/storage/mirror-001.png",
        "https://i.imgur.com/abc123.png",
        2048,
        "image/png",
        "sha256:deadbeef",
        "operator-x",
    )
    .await
    .expect("insert mirror");

    let mirrors = media_resilience::list_local_mirrors(db, reference_id)
        .await
        .expect("list mirrors");
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].id, mirror_id);
    assert_eq!(mirrors[0].storage_path, "/storage/mirror-001.png");
    assert_eq!(mirrors[0].checksum_sha256, "sha256:deadbeef");

    media_resilience::deactivate_local_mirror(db, mirror_id)
        .await
        .expect("deactivate");

    let mirrors = media_resilience::list_local_mirrors(db, reference_id)
        .await
        .expect("list mirrors");
    assert_eq!(mirrors.len(), 0);
}

#[tokio::test]
async fn ipfs_pin_insert_and_list() {
    let dir = test_support::scratch_dir("ma_ipfs");
    let tdb = test_support::TestDb::connect_with_dir("ma-ipfs", &dir).await;
    let db = tdb.db();

    let pin_id = "pin-001";
    let reference_id = "ref-001";

    media_resilience::insert_ipfs_pin(
        db, pin_id, reference_id,
        "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
        "pinata",
        4096,
    )
    .await
    .expect("insert ipfs pin");

    let pins = media_resilience::list_ipfs_pins(db, reference_id)
        .await
        .expect("list ipfs pins");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].id, pin_id);
    assert_eq!(pins[0].pin_service, "pinata");
    assert_eq!(pins[0].file_size_bytes, 4096);
}

#[tokio::test]
async fn dmca_takedown_file_and_resolve() {
    let dir = test_support::scratch_dir("ma_dmca");
    let tdb = test_support::TestDb::connect_with_dir("ma-dmca", &dir).await;
    let db = tdb.db();

    let mirror_id = "mirror-002";
    let reference_id = "ref-002";

    media_resilience::insert_local_mirror(
        db, mirror_id, reference_id,
        "/storage/dmca-test.png",
        "https://i.imgur.com/dmca.png",
        1024,
        "image/png",
        "sha256:dmcaca",
        "operator-y",
    )
    .await
    .expect("insert mirror for dmca");

    let takedown_id = "td-001";

    media_resilience::file_dmca_takedown(
        db, takedown_id, mirror_id,
        "Copyright Holder",
        "copyright@example.com",
        "Original artwork by me",
        "This mirror copies my work without permission",
    )
    .await
    .expect("file dmca");

    media_resilience::resolve_dmca_takedown(db, takedown_id, true, "admin-z")
        .await
        .expect("resolve dmca");

    let takedown_id_2 = "td-002";
    media_resilience::file_dmca_takedown(
        db, takedown_id_2, mirror_id,
        "Another Claimant",
        "another@example.com",
        "My photo",
        "Unauthorized use",
    )
    .await
    .expect("file dmca 2");

    media_resilience::resolve_dmca_takedown(db, takedown_id_2, false, "admin-w")
        .await
        .expect("reject dmca");
}
