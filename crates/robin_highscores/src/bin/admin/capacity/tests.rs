use super::*;

#[tokio::test]
async fn backup_space_topology_densifies_sparse_and_tiny_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("objects");
    tokio::fs::create_dir(&root).await.unwrap();
    let allocation = 4096;
    let sparse = std::fs::File::create(root.join("sparse")).unwrap();
    sparse.set_len(allocation + 1).unwrap();
    drop(sparse);
    for index in 0..128 {
        tokio::fs::write(root.join(format!("tiny-{index:03}")), [0x5a])
            .await
            .unwrap();
    }

    let mut topology = BackupCopyTopology::default();
    add_regular_tree_capacity(&root, allocation, &mut topology)
        .await
        .unwrap();
    assert_eq!(topology.regular_files, 129);
    assert_eq!(topology.directories, 1);
    assert_eq!(
        topology.dense_file_bytes,
        2 * allocation + 128 * allocation,
        "logical apparent bytes must be rounded as dense destination allocations"
    );
    assert!(
        topology.dense_file_bytes > allocation + 1 + 128,
        "the strict estimate must not collapse to du --bytes semantics"
    );
    assert_eq!(round_up_to_allocation(0, allocation).unwrap(), 0);
    assert_eq!(round_up_to_allocation(1, allocation).unwrap(), allocation);
    assert_eq!(
        round_up_to_allocation(allocation + 1, allocation).unwrap(),
        2 * allocation
    );
    assert!(round_up_to_allocation(u64::MAX, allocation).is_err());
    let million_tiny_files = 1_000_000_u64;
    let million_dense_bytes = round_up_to_allocation(1, allocation)
        .unwrap()
        .checked_mul(million_tiny_files)
        .unwrap();
    let million_entry_bytes = conservative_entry_overhead(million_tiny_files, allocation).unwrap();
    assert_eq!(million_dense_bytes, 4_096_000_000);
    assert_eq!(million_entry_bytes, 4_096_000_000);
    assert!(
        conservative_entry_overhead(u64::MAX, allocation).is_err(),
        "a hostile topology must fail closed on arithmetic overflow"
    );
}
