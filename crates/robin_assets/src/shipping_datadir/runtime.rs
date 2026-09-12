//! Shipping runtime boundary; payload wire shapes remain in the parent.
use super::*;

impl ShippingDatadir {
    /// Iterate installed language packs in stable canonical-locale order.
    pub fn available_locales(&self) -> impl Iterator<Item = (&str, &ShippingLocale)> {
        self.locales
            .iter()
            .map(|(locale, assets)| (locale.as_str(), assets))
    }

    /// Resolve a canonical locale, Windows LCID, or retained alias to a pack.
    /// Invalid identifiers are errors; absent packs return `Ok(None)`.
    pub fn locale(&self, locale: &str) -> Result<Option<&ShippingLocale>> {
        if let Some((_, assets)) = self.locales.iter().find(|(_, assets)| {
            assets
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(locale.trim()))
        }) {
            return Ok(Some(assets));
        }
        let canonical = canonical_locale_id(locale)?;
        Ok(self.locales.get(&canonical))
    }

    /// Whether this manifest predates explicit locale metadata. Its top-level
    /// resources remain usable, but their language identity is unknowable and
    /// must not be advertised as a specific installed locale.
    pub fn is_legacy_single_locale(&self) -> bool {
        self.locales.is_empty()
    }

    pub fn locale_resource(&self, locale: &str, path: &str) -> Result<Option<&ResourceManager>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.res_files.get(&key)))
    }

    pub fn locale_pak(&self, locale: &str, path: &str) -> Result<Option<&[EncodedPicture]>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.pak_files.get(&key))
            .map(Vec::as_slice))
    }

    pub fn locale_level_descriptors(
        &self,
        locale: &str,
        path: &str,
    ) -> Result<Option<&LevelDescriptors>> {
        let key = canonical_shipping_asset_key(path);
        let filename = key.rsplit('/').next().unwrap_or(&key);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.red_files.get(filename)))
    }

    pub fn locale_raw(&self, locale: &str, path: &str) -> Result<Option<&[u8]>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.raw.get(&key))
            .map(Vec::as_slice))
    }

    /// Return a shareable VFS overlay for a locale. The first call clones the
    /// serialized raw map into an `Arc`; subsequent calls share that allocation.
    pub fn locale_bundle(&self, locale: &str) -> Result<Option<Arc<robin_util::asset_fs::Bundle>>> {
        let canonical = if let Some((canonical, _)) = self.locales.iter().find(|(_, assets)| {
            assets
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(locale.trim()))
        }) {
            canonical.clone()
        } else {
            canonical_locale_id(locale)?
        };
        let Some(assets) = self.locales.get(&canonical) else {
            return Ok(None);
        };
        if let Some(cached) = self
            .runtime
            .locale_bundle_cache
            .read()
            .expect("shipping locale bundle cache lock poisoned")
            .get(&canonical)
            .cloned()
        {
            return Ok(Some(cached));
        }

        // A locale is a complete text/UI overlay. English is only a fallback
        // for the two explicitly optional presentation families; allowing it
        // to fill arbitrary missing files would create a partly translated UI
        // and conceal an incomplete pack.
        let mut raw = self
            .locales
            .get("en-US")
            .filter(|_| canonical != "en-US")
            .map(|english| {
                english
                    .raw
                    .iter()
                    .filter(|(key, _)| is_optional_english_fallback_key(key))
                    .map(|(key, bytes)| (key.clone(), bytes.clone().into()))
                    .collect::<robin_util::asset_fs::Bundle>()
            })
            .unwrap_or_default();
        raw.extend(
            assets
                .raw
                .iter()
                .map(|(key, bytes)| (key.clone(), bytes.clone().into())),
        );
        let bundle = Arc::new(raw);
        let bundle = self
            .runtime
            .locale_bundle_cache
            .write()
            .expect("shipping locale bundle cache lock poisoned")
            .entry(canonical)
            .or_insert_with(|| bundle.clone())
            .clone();
        Ok(Some(bundle))
    }

    /// Atomically select the parsed and raw resources used by runtime
    /// language-aware loaders. `None` clears the locale overlay for legacy
    /// single-language manifests.
    pub fn set_active_locale(&self, locale: Option<&str>) -> Result<()> {
        let (canonical, bundle) = match locale {
            Some(locale) => {
                let canonical = canonical_locale_id(locale)?;
                if !self.locales.contains_key(&canonical) {
                    return Err(anyhow!("shipping locale {canonical} is not installed"));
                }
                let bundle = self
                    .locale_bundle(&canonical)?
                    .ok_or_else(|| anyhow!("shipping locale {canonical} has no raw bundle"))?;
                (Some(canonical), Some(bundle))
            }
            None => (None, None),
        };

        self.asset_vfs()
            .select_locale(canonical, bundle)
            .context("install shipping locale bundle")?;
        Ok(())
    }

    pub fn active_locale_name(&self) -> Option<String> {
        self.asset_vfs().selection_snapshot().locale
    }

    /// Capture selection once for the entire lookup. An installed active
    /// identity must resolve; only an unselected locale or an absent asset is
    /// optional. The borrowed pack stays valid if selection changes later.
    pub(super) fn active_locale(&self) -> Option<&ShippingLocale> {
        let name = self.active_locale_name()?;
        Some(
            self.locale(&name)
                .unwrap_or_else(|error| {
                    panic!("invalid active shipping locale {name:?}: {error:#}")
                })
                .unwrap_or_else(|| panic!("active shipping locale {name:?} is not installed")),
        )
    }

    pub fn active_resource(&self, path: &str) -> Option<&ResourceManager> {
        if !is_locale_overlay_key(path) {
            return None;
        }
        let locale = self.active_locale()?;
        locale.res_files.get(&canonical_shipping_asset_key(path))
    }

    pub fn active_pak(&self, path: &str) -> Option<&[EncodedPicture]> {
        let locale = self.active_locale()?;
        locale
            .pak_files
            .get(&canonical_shipping_asset_key(path))
            .map(Vec::as_slice)
    }

    pub fn localized_pak(&self, path: &str) -> Option<&[EncodedPicture]> {
        self.localized_pak_for_locale(path, self.active_locale())
    }

    pub(super) fn localized_pak_for_locale<'a>(
        &'a self,
        path: &str,
        locale: Option<&'a ShippingLocale>,
    ) -> Option<&'a [EncodedPicture]> {
        let key = canonical_shipping_asset_key(path);
        if let Some(locale) = locale {
            // PAKs under locale roots commonly bake translated titles into
            // their pixels. A v5 manifest must not substitute the top-level
            // compatibility pack when the selected locale omitted one.
            locale.pak_files.get(&key).map(Vec::as_slice)
        } else {
            self.pak_files.get(&key).map(Vec::as_slice)
        }
    }

    pub fn active_level_descriptors(&self, path: &str) -> Option<&LevelDescriptors> {
        let locale = self.active_locale()?;
        let key = canonical_shipping_asset_key(path);
        let filename = key.rsplit('/').next().unwrap_or(&key);
        locale.red_files.get(filename)
    }

    /// Resolve locale-specific metadata first, then the shared descriptor.
    /// Stock Demo installs put the RED table indices in shared Data/Text while
    /// Level.res strings live in the locale overlay. Reusing those indices is
    /// not permission to fall back to another locale's strings or resources.
    pub fn localized_level_descriptors(&self, path: &str) -> Option<&LevelDescriptors> {
        self.localized_level_descriptors_for_locale(path, self.active_locale())
    }

    pub(super) fn localized_level_descriptors_for_locale<'a>(
        &'a self,
        path: &str,
        locale: Option<&'a ShippingLocale>,
    ) -> Option<&'a LevelDescriptors> {
        let key = canonical_shipping_asset_key(path);
        let filename = key.rsplit('/').next().unwrap_or(&key);
        if let Some(descriptor) = locale.and_then(|assets| assets.red_files.get(filename)) {
            return Some(descriptor);
        }
        // Existing shipping producers retain the original mixed-case RED
        // filename in the shared map; locale maps use canonical keys. Accept
        // both without changing the payload format or regenerating stock data.
        let mut matches = self
            .red_files
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(filename));
        let (_, descriptor) = matches.next()?;
        assert!(
            matches.next().is_none(),
            "ambiguous shared level descriptor {filename}"
        );
        if locale.is_some()
            && (descriptor.custom_popup_texts.iter().any(Option::is_some)
                || descriptor
                    .custom_short_briefings
                    .iter()
                    .any(Option::is_some)
                || descriptor.custom_dialogue_texts.iter().any(Option::is_some))
        {
            tracing::warn!(
                "shared descriptor {filename} contains authored text; refusing cross-locale fallback"
            );
            return None;
        }
        Some(descriptor)
    }

    /// Localized profile metadata for presentation lookups such as mission
    /// titles. Engine construction must use the language-independent top-level
    /// `profiles` index so a client locale cannot alter simulation data.
    pub fn active_profiles(&self) -> Option<&ProfileManager> {
        self.active_locale()?.profiles.as_ref()
    }

    /// Parse a shipping datadir blob: zstd decompress + native bitcode decode.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let compressed = robin_util::asset_fs::read_shared(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.runtime.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Load through an explicit VFS instance.
    pub fn load_from_vfs(vfs: &robin_util::asset_fs::AssetVfs, path: &Path) -> Result<Self> {
        let compressed = vfs
            .read_shared(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.runtime.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Parse a shipping datadir blob already in memory.  Used by the
    /// wasm-bindgen bootstrap, which fetches `datadir.bin` from JS,
    /// hands the bytes to Rust, and decodes here — bypassing the
    /// `asset_fs::read_shared` path (which is bundle-only on wasm and the
    /// bundle isn't installed yet at this point).
    pub fn from_compressed_bytes(compressed: &[u8]) -> Result<Self> {
        // Streaming decoder with `windowLogMax=30` (1 GiB virtual) —
        // the cap zstd permits on 32-bit builds like wasm32. Shipping
        // blobs destined for wasm must be compressed with
        // `window_log <= 30` (the desktop encoder uses 31, which zstd
        // rejects on 32-bit targets — see `zstd_max_compress`).
        let blob = zstd_decompress(compressed)?;
        let dd = decode_native(&blob)?;
        tracing::info!(
            "loaded shipping datadir ({} → {} bytes)",
            compressed.len(),
            blob.len()
        );
        Ok(dd)
    }

    pub fn set_remote_base_url(&mut self, url: String) {
        self.runtime.remote_base_url = Some(url.trim_end_matches('/').to_owned());
    }

    pub fn remote_base_url(&self) -> Option<&str> {
        self.runtime.remote_base_url.as_deref()
    }

    pub fn mission_ref(&self, mission: &str) -> Option<&ShippingMissionRef> {
        self.missions.get(mission)
    }

    pub fn has_mission(&self, mission: &str) -> bool {
        self.missions.contains_key(mission) || self.levels.contains_key(mission)
    }

    pub fn source_file_path(&self, relative: &str) -> Result<PathBuf> {
        let source_dir = self.runtime.source_dir.as_ref().ok_or_else(|| {
            anyhow!("shipping manifest has no native source directory for {relative}")
        })?;
        Ok(source_dir.join(relative))
    }

    pub fn is_mission_loaded(&self, mission: &str) -> bool {
        self.runtime
            .loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .contains_key(mission)
    }

    pub fn cache_preloaded_file(&self, file: String, bytes: Vec<u8>) -> Result<()> {
        let mut files = self
            .runtime
            .preloaded_files
            .write()
            .expect("shipping preloaded-file lock poisoned");
        match files.entry(file) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Arc::new(bytes));
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(entry) => Err(anyhow!(
                "shipping file {:?} was already preloaded",
                entry.key()
            )),
        }
    }

    pub fn preloaded_file(&self, file: &str) -> Option<Arc<Vec<u8>>> {
        self.runtime
            .preloaded_files
            .read()
            .expect("shipping preloaded-file lock poisoned")
            .get(file)
            .cloned()
    }

    pub fn install_mission(&self, mission: &str, payload: ShippingMission) -> Result<()> {
        if !payload.levels.contains_key(mission) {
            return Err(anyhow!(
                "shipping payload for {mission} does not contain its level"
            ));
        }
        if let (Some(base), Some(bank)) = (self.sprite_bank.as_ref(), payload.sprite_bank.as_ref())
            && (base.signature != bank.signature || base.sprite_count != bank.sprite_count)
        {
            return Err(anyhow!(
                "shipping mission {mission} sprite bank is incompatible with boot dictionaries"
            ));
        }
        let prepared = payload.prepare(mission)?;
        let mut loaded = self
            .runtime
            .loaded_missions
            .write()
            .expect("shipping mission lock poisoned");
        // Prepare and validate before replacing the retained parsed generation.
        self.publish_mission(mission, &prepared.mission)?;
        for previous in loaded.values() {
            previous.sprite_streaming.retire();
        }
        loaded.clear();
        loaded.insert(mission.to_owned(), Arc::new(prepared.mission));
        Ok(())
    }

    pub fn install_mission_parts(
        &self,
        mission: &str,
        parts: impl IntoIterator<Item = ShippingMission>,
    ) -> Result<()> {
        let mut merged = ShippingMission::default();
        for part in parts {
            merged.merge_from(part)?;
        }
        self.install_mission(mission, merged)
    }

    /// Synchronous native-file equivalent of the runtime's asynchronous
    /// mission loader. Developer tools use this when they open a converted
    /// datadir directly rather than entering the game session boundary.
    pub fn load_mission_from_source(&self, mission: &str) -> Result<()> {
        if self.is_mission_loaded(mission) {
            return self.activate_mission(mission);
        }
        let reference = self
            .mission_ref(mission)
            .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
        let mut merged = ShippingMission::default();
        for file in &reference.files {
            let path = self.source_file_path(file)?;
            let compressed = self
                .asset_vfs()
                .read_shared(&path)
                .with_context(|| format!("read {}", path.display()))?;
            merged.merge_part(
                decode_mission_compressed(&compressed)
                    .with_context(|| format!("decode {}", path.display()))?,
            )?;
        }
        self.install_mission_parts(mission, std::iter::once(merged))
    }

    pub fn activate_mission(&self, mission: &str) -> Result<()> {
        let loaded = self
            .runtime
            .loaded_missions
            .write()
            .expect("shipping mission lock poisoned");
        let payload = loaded
            .get(mission)
            .ok_or_else(|| anyhow!("shipping mission {mission} has not been loaded"))?;
        self.publish_mission(mission, payload)
    }

    /// Called with the loaded-mission lock held. Readers acquire that lock
    /// before capturing raw selection, so parsed and raw generations agree.
    pub(super) fn publish_mission(&self, mission: &str, payload: &ShippingMission) -> Result<()> {
        let raw = payload
            .raw_bundle
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("shipping mission {mission} has no installed raw bundle"))?;
        let selection = self.selection_snapshot();
        if selection.mission.as_deref() == Some(mission)
            && selection
                .active_bundle
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &raw))
        {
            return Ok(());
        }
        let raw_files = raw.len();
        let rhs_files = payload.rhs_files.len();
        self.asset_vfs()
            .select_mission(Some(mission.to_owned()), raw.clone())
            .context("mount shipping mission assets")?;
        tracing::info!(mission, raw_files, rhs_files, "activated shipping mission");
        Ok(())
    }

    pub fn active_mission_payload(&self) -> Option<Arc<ShippingMission>> {
        self.mission_selection_snapshot().1
    }

    /// Capture a closed execution catalog for this selected mission. Later
    /// activations (here or in another installation) cannot change its RHS.
    /// Missing parsed shipping entries never fall through to a global datadir.
    pub fn mission_resource_environment(
        &self,
        mission_name: &str,
    ) -> Result<Arc<robin_engine::sprite_script::MissionResourceEnvironment>> {
        let (selection, mission) = self.mission_selection_snapshot();
        let mission =
            if selection.mission.as_deref() == Some(mission_name) {
                Some(mission.ok_or_else(|| {
                    anyhow!("selected mission {mission_name} has no parsed payload")
                })?)
            } else if self.levels.contains_key(mission_name) {
                None // Valid monolithic v15 payload: root fields own this mission.
            } else {
                return Err(anyhow!(
                    "cannot prepare {mission_name}: selected mission is {:?}",
                    selection.mission
                ));
            };
        let mut files = self.rhs_files.iter().collect::<BTreeMap<_, _>>();
        // Preserve selected-mission priority over boot resources.
        if let Some(mission) = &mission {
            files.extend(mission.rhs_files.iter());
        }
        let resources = robin_engine::sprite_script::MissionResourceEnvironment::default()
            .with_parsed_rhs(
                files
                    .into_iter()
                    .map(|(path, rhs)| (path.as_str(), rhs.signature, rhs.profiles.as_slice())),
            )
            .map_err(anyhow::Error::msg)?;
        let scripts = mission
            .as_ref()
            .map(|mission| &mission.payload.scripts)
            .unwrap_or(&self.scripts);
        let programs = scripts
            .iter()
            .map(|(name, scb)| {
                robin_engine::script_manager::ScriptProgram::from_scb(scb.clone())
                    .map(|program| (name.clone(), Arc::new(program)))
                    .map_err(|error| anyhow!("prepare mission script {name}: {error}"))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Arc::new(resources.with_programs(programs)))
    }

    /// Pin the parsed mission and its raw VFS generation together. Consumers
    /// performing several related reads can retain this pair across switches.
    pub fn mission_selection_snapshot(
        &self,
    ) -> (
        robin_util::asset_fs::AssetSelection,
        Option<Arc<ShippingMission>>,
    ) {
        let loaded = self
            .runtime
            .loaded_missions
            .read()
            .expect("shipping mission lock poisoned");
        let selection = self.selection_snapshot();
        let payload = selection.mission.as_ref().map(|mission| {
            loaded
                .get(mission)
                .unwrap_or_else(|| panic!("selected shipping mission {mission} is not loaded"))
                .clone()
        });
        (selection, payload)
    }

    pub fn loaded_mission(&self, mission: &str) -> Option<Arc<ShippingMission>> {
        self.runtime
            .loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .get(mission)
            .cloned()
    }

    pub fn loaded_mission_count(&self) -> usize {
        self.runtime
            .loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .len()
    }

    pub fn loaded_level(&self, mission: &str) -> Option<LoadedLevel> {
        self.loaded_mission(mission)
            .and_then(|payload| payload.levels.get(mission).cloned())
            .or_else(|| self.levels.get(mission).cloned())
    }

    pub fn mission_scripts(&self, mission: &str) -> BTreeMap<String, ScbFile> {
        self.loaded_mission(mission)
            .map(|payload| payload.scripts.clone())
            .unwrap_or_else(|| self.scripts.clone())
    }

    pub fn with_active_sprite_bank<R>(
        &self,
        use_bank: impl FnOnce(
            &ShippingSpriteBank,
            &[FrameDictionary],
            Option<&crate::late_sprites::SpriteStreaming>,
        ) -> R,
    ) -> Option<R> {
        let loaded = self.active_mission_payload();
        let bank = loaded
            .as_ref()
            .and_then(|mission| mission.sprite_bank.as_ref())
            .or(self.sprite_bank.as_ref())?;
        let dictionaries = if bank.dictionaries.is_empty() {
            &self.sprite_bank.as_ref()?.dictionaries
        } else {
            &bank.dictionaries
        };
        Some(use_bank(
            bank,
            dictionaries,
            loaded.as_ref().map(|mission| mission.sprite_streaming()),
        ))
    }

    pub fn active_mission_name(&self) -> Option<String> {
        self.asset_vfs().selection_snapshot().mission
    }

    /// Publish the exact speech-profile closure selected at the asynchronous
    /// mission boundary. Process-wide audio caches use this instead of
    /// scanning every CPF actor and warning for intentionally unmounted data.
    pub fn set_active_exclamation_ids(&self, ids: BTreeSet<u32>) {
        let mut active = self
            .runtime
            .active_exclamation_ids
            .write()
            .expect("shipping active exclamation lock poisoned");
        if *active != ids {
            *active = ids;
            self.asset_vfs().invalidate_content(true);
        }
    }

    pub fn active_exclamation_ids(&self) -> Vec<u32> {
        self.runtime
            .active_exclamation_ids
            .read()
            .expect("shipping active exclamation lock poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Return the source-authoritative duration for boot or active-mission
    /// audio. Web artifacts use `.opus` keys even though legacy metadata asks
    /// for `.wav` or `.ogg`, so resolution includes that target extension.
    pub fn active_audio_duration_ms(&self, path: &Path) -> Option<u32> {
        self.active_audio_metadata(path)
            .map(|(_, duration)| duration)
    }

    /// Return encoded byte size and source duration without copying the VFS
    /// asset. The wasm sound cache only needs this bookkeeping because Web
    /// Audio owns both decoding and PCM playback storage.
    pub fn active_audio_metadata(&self, path: &Path) -> Option<(u32, u32)> {
        if let Some(asset) = self.find_audio_asset(path) {
            return Some((asset.encoded_size, asset.duration_ms));
        }
        let key = robin_util::asset_fs::bundle_key(path);
        let opus = Path::new(&key)
            .with_extension("opus")
            .to_string_lossy()
            .replace('\\', "/");
        let mission = self.active_mission_payload().and_then(|payload| {
            let duration = payload
                .audio_durations_ms
                .get(&key)
                .or_else(|| payload.audio_durations_ms.get(&opus))
                .copied()?;
            let bytes = payload.raw_bundle.get()?.get(&key).or_else(|| {
                payload
                    .raw_bundle
                    .get()
                    .and_then(|bundle| bundle.get(&opus))
            })?;
            Some((u32::try_from(bytes.len()).ok()?, duration))
        });
        mission.or_else(|| {
            let duration = self
                .audio_durations_ms
                .get(&key)
                .or_else(|| self.audio_durations_ms.get(&opus))
                .copied()?;
            let bytes = self.raw_asset(&key).or_else(|| self.raw_asset(&opus))?;
            Some((u32::try_from(bytes.len()).ok()?, duration))
        })
    }

    /// Resolve legacy engine paths to a standalone browser audio asset.
    ///
    /// Callers may supply source extensions, bare sound names, paths relative
    /// to `Sounds/Exclamations`, or native absolute paths containing a `Data`
    /// component. All aliases resolve to the one catalog entry and therefore
    /// the same content URL/browser decode cache entry.
    pub fn remote_audio_asset(&self, path: &Path) -> Option<RemoteAudioAsset> {
        let asset = self.find_audio_asset(path)?;
        let base = self.runtime.remote_base_url.as_deref()?;
        Some(RemoteAudioAsset {
            url: format!("{}/{}", base.trim_end_matches('/'), asset.file),
            encoded_size: asset.encoded_size,
            duration_ms: asset.duration_ms,
            bundle_offset: asset.bundle_offset,
        })
    }

    /// Catalog keys required before any mission is selected (menu effects
    /// and menu music). Browser startup decodes this deliberately small set;
    /// it must not infer boot membership by scanning the whole catalog.
    pub fn boot_audio_keys(&self) -> Vec<String> {
        self.audio_durations_ms
            .keys()
            .filter(|key| self.audio_assets.contains_key(*key))
            .cloned()
            .collect()
    }

    /// Catalog keys the ACTIVE mission's payloads reference (its dialogue,
    /// required actor voices, music, ambience): the exact mission warmup set.
    pub fn active_audio_keys(&self) -> Vec<String> {
        let Some(payload) = self.active_mission_payload() else {
            return Vec::new();
        };
        payload
            .audio_durations_ms
            .keys()
            .filter(|key| self.audio_assets.contains_key(*key))
            .cloned()
            .collect()
    }

    pub(super) fn find_audio_asset(&self, path: &Path) -> Option<&ShippingAudioAsset> {
        audio_lookup_keys(path)
            .into_iter()
            .find_map(|key| self.audio_assets.get(&key))
    }

    /// Borrow one boot asset whether installation has moved it into the VFS
    /// shared-byte bundle or this manifest is still in converter/tool form.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.runtime
                .installed
                .get()
                .and_then(|installed| installed.boot_raw_bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }
}

/// English substitution is deliberately limited to optional recorded media.
/// All matching uses canonical shipping keys (relative to `Data/`).
pub fn is_optional_english_fallback_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    robin_util::asset_fs::is_optional_english_fallback_key(&key)
}

/// Text resources are required to come from the selected locale. They never
/// fall through to the compatibility/default maps of a shipping manifest.
pub fn is_required_locale_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    robin_util::asset_fs::is_required_locale_key(&key)
}

/// Locale selection is presentation-only. Even if a converted locale source
/// contains a complete Data tree, it must never replace simulation inputs such
/// as levels, scripts, or gameplay profile data.
pub fn is_locale_overlay_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    robin_util::asset_fs::is_locale_overlay_key(&key)
}

impl ShippingMission {
    pub fn from_payload(payload: ShippingMissionPayload) -> Self {
        Self {
            payload,
            raw_bundle: OnceLock::new(),
            sprite_streaming: Default::default(),
        }
    }

    /// Runtime handoff for an installed mission, never an independently decoded part.
    pub fn sprite_streaming(&self) -> &crate::late_sprites::SpriteStreaming {
        assert!(
            self.raw_bundle.get().is_some(),
            "sprite streaming requires an installed mission"
        );
        &self.sprite_streaming
    }

    /// Consume staging ownership before publishing runtime data. No installed
    /// mission or VFS generation is mutated if decoding/validation fails.
    pub(super) fn prepare(mut self, mission: &str) -> Result<PreparedShippingMission> {
        if let Some(bank) = self.payload.sprite_bank.as_mut() {
            bank.materialize_vq_chunks(&self.payload.rhs_files)
                .with_context(|| format!("materialize VQ sprite chunks for mission {mission}"))?;
            bank.materialize_rle_jxl_chunks().with_context(|| {
                format!("materialize RLE-JXL sprite chunks for mission {mission}")
            })?;
        }
        let raw = std::mem::take(&mut self.payload.raw)
            .into_iter()
            .map(|(path, bytes)| (path, bytes.into()))
            .collect();
        self.raw_bundle
            .set(Arc::new(raw))
            .map_err(|_| anyhow!("shipping mission {mission} raw bundle was already installed"))?;

        Ok(PreparedShippingMission { mission: self })
    }

    /// Borrow an installed raw asset without copying its encoded bytes.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.raw_bundle
                .get()
                .and_then(|bundle| bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }

    /// Move-merge one independently decoded dependency into this payload.
    /// Loaders use this incrementally so compressed/decoded part shells can be
    /// released as soon as each bounded fetch completes.
    pub fn merge_part(&mut self, source: Self) -> Result<()> {
        self.merge_from(source)
    }

    pub(super) fn merge_from(&mut self, source: Self) -> Result<()> {
        if self.raw_bundle.get().is_some() || source.raw_bundle.get().is_some() {
            bail!("cannot merge a prepared shipping mission");
        }
        // Independently bounded parts must not assemble an unbounded raw
        // bundle. This is resident payload accounting, not decoder scratch.
        let mut raw_bytes = 0usize;
        for bytes in self.raw.values().chain(source.raw.values()) {
            raw_bytes = raw_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| anyhow!("shipping mission raw bundle size overflow"))?;
            if raw_bytes > 1024 * 1024 * 1024 {
                bail!("shipping mission raw bundle exceeds 1 GiB");
            }
        }
        let mut source = source.payload;
        merge_unique_owned(&mut self.levels, source.levels, "level")?;
        merge_unique_owned(&mut self.scripts, source.scripts, "script")?;
        merge_unique_owned(&mut self.rhs_files, source.rhs_files, "RHS")?;
        merge_unique_owned(&mut self.raw, source.raw, "raw asset")?;
        merge_unique_owned(
            &mut self.audio_durations_ms,
            source.audio_durations_ms,
            "audio duration",
        )?;
        let Some(mut source_bank) = source.sprite_bank.take() else {
            return Ok(());
        };
        let bank = self.sprite_bank.get_or_insert_with(|| ShippingSpriteBank {
            signature: source_bank.signature,
            dictionaries: std::mem::take(&mut source_bank.dictionaries),
            sprite_count: source_bank.sprite_count,
            sprites: Vec::new(),
            vq_chunks: Vec::new(),
            rle_jxl_chunks: Vec::new(),
        });
        if bank.signature != source_bank.signature || bank.sprite_count != source_bank.sprite_count
        {
            return Err(anyhow!("shipping sprite-bank parts are incompatible"));
        }
        if bank.dictionaries.is_empty() {
            bank.dictionaries = std::mem::take(&mut source_bank.dictionaries);
        } else if !source_bank.dictionaries.is_empty()
            && bitcode::encode(&bank.dictionaries) != bitcode::encode(&source_bank.dictionaries)
        {
            return Err(anyhow!("shipping sprite-bank dictionaries conflict"));
        }
        bank.vq_chunks.append(&mut source_bank.vq_chunks);
        bank.rle_jxl_chunks.append(&mut source_bank.rle_jxl_chunks);
        for (index, sprite) in source_bank.sprites {
            if index >= bank.sprite_count {
                return Err(anyhow!(
                    "shipping sprite-bank part contains out-of-range sprite {index} (bank has {} slots)",
                    bank.sprite_count
                ));
            }
            match bank
                .sprites
                .binary_search_by_key(&index, |(index, _)| *index)
            {
                Ok(position) => {
                    let existing = &bank.sprites[position].1;
                    // The streaming wasm loader materializes VQ grids while
                    // later parts are still downloading, so `existing` may
                    // already carry a decoded grid whose incoming twin is
                    // still the empty-`packed_data` VQ placeholder. Compare
                    // with the materialized side blanked; any chunk that
                    // decodes this sprite again still proves grid equality
                    // in `apply_decoded_vq_chunk`. Native installs merge
                    // strictly before materialization, where this branch
                    // cannot trigger.
                    let blanked;
                    let comparable =
                        if !existing.packed_data.is_empty() && sprite.packed_data.is_empty() {
                            blanked = ShippingSprite {
                                packed_data: Arc::new(Vec::new()),
                                ..existing.clone()
                            };
                            &blanked
                        } else {
                            existing
                        };
                    if bitcode::encode(comparable) != bitcode::encode(&sprite) {
                        return Err(anyhow!(
                            "shipping sprite-bank parts conflict at sprite {index}"
                        ));
                    }
                }
                Err(position) => bank.sprites.insert(position, (index, sprite)),
            }
        }
        bank.validate_resident_budget()
    }
}

fn merge_unique_owned<K, V>(dst: &mut BTreeMap<K, V>, src: BTreeMap<K, V>, kind: &str) -> Result<()>
where
    K: Ord + std::fmt::Debug,
{
    for (key, value) in src {
        if dst.contains_key(&key) {
            return Err(anyhow!("duplicate shipping {kind} key {key:?}"));
        }
        dst.insert(key, value);
    }
    Ok(())
}

pub(super) fn audio_lookup_keys(path: &Path) -> Vec<String> {
    let mut raw = path.to_string_lossy().replace('\\', "/");
    while let Some(rest) = raw.strip_prefix("./") {
        raw = rest.to_owned();
    }
    let lowercase = raw.to_ascii_lowercase();
    let key = if let Some(index) = lowercase.find("/data/") {
        raw[index + "/data/".len()..].to_owned()
    } else if lowercase.starts_with("data/") {
        raw["data/".len()..].to_owned()
    } else {
        raw.trim_start_matches('/').to_owned()
    }
    .to_ascii_lowercase();

    let mut bases = vec![key.clone()];
    if let Some(rest) = key.strip_prefix("exclamations/") {
        bases.push(format!("sounds/exclamations/{rest}"));
    }
    if !key.starts_with("sounds/") && !key.starts_with("musics/") {
        bases.push(format!("sounds/{key}"));
        bases.push(format!("sounds/exclamations/{key}"));
    }

    let mut keys = Vec::with_capacity(bases.len() * 2);
    for base in bases {
        keys.push(base.clone());
        let opus = Path::new(&base)
            .with_extension("opus")
            .to_string_lossy()
            .replace('\\', "/");
        if opus != base {
            keys.push(opus);
        }
    }
    keys
}
