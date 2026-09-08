#[cfg(test)]
mod wire_contract {
    use super::*;
    #[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
    struct FrozenResourceWire {
        /// Picture collections keyed by resource ID.
        pictures: HashMap<ResourceId, Vec<Option<Picture>>>,
        /// Shipping-only compressed picture collections keyed by resource ID.
        #[serde(default)]
        encoded_pictures: HashMap<ResourceId, Vec<Option<EncodedPicture>>>,
        /// Mouse cursor metadata.
        mouse_entries: HashMap<ResourceId, MouseEntry>,
        /// Wide-string tables.
        strings: HashMap<ResourceId, Vec<String>>,
        /// Wave/sound-path tables.
        waves: HashMap<ResourceId, Vec<String>>,
        /// Reference counts per resource.
        references: HashMap<ResourceId, u32>,
        /// On-disk locations for recovery after dismiss.
        file_entries: HashMap<ResourceId, ResourceFileEntry>,
        /// Parsed shipping resources deliberately omit their legacy archive. They
        /// must never silently attempt to recover dismissed entries from a raw
        /// `.res` file that was not shipped.
        #[serde(default)]
        recovery_disabled: bool,
    }

    #[test]
    fn resident_and_lifetime_split_preserves_legacy_wire_and_json() {
        let mut manager = ResourceManager::with_files(Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ))));
        manager.data.pictures.insert(7, vec![None]);
        manager.data.encoded_pictures.insert(8, vec![None]);
        manager.data.strings.insert(9, vec!["fixture".into()]);
        manager.data.waves.insert(10, vec!["sound.wav".into()]);
        manager.lifetime.references.insert(9, 3);
        manager.lifetime.file_entries.insert(
            9,
            ResourceFileEntry {
                file_path: "fixture.res".into(),
                file_offset: 24,
                resource_type: *b"STR ",
            },
        );
        let json = serde_json::to_value(&manager).unwrap();
        let frozen: FrozenResourceWire = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(bitcode::encode(&manager), bitcode::encode(&frozen));
        let decoded: ResourceManager = bitcode::decode(&bitcode::encode(&frozen)).unwrap();
        assert!(decoded.files.is_none());
        assert_eq!(serde_json::to_value(&decoded).unwrap(), json);
        assert_eq!(decoded.data.strings[&9], ["fixture"]);
        assert_eq!(decoded.lifetime.references[&9], 3);
        assert_eq!(decoded.lifetime.file_entries[&9].file_offset, 24);
        manager.disable_recovery_for_shipping();
        let frozen: FrozenResourceWire =
            serde_json::from_value(serde_json::to_value(&manager).unwrap()).unwrap();
        assert_eq!(bitcode::encode(&manager), bitcode::encode(&frozen));
        assert!(
            manager
                .recover_resource(9)
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
    }
}
