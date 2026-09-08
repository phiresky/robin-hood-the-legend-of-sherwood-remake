//! Exact eager opacity for experimental sprite-grid streaming.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct SpriteOpacity {
    pub bank_id: u32,
    pub width: u16,
    pub height: u16,
    pub dictionary_index: u16,
    /// Row-major pixels, least significant bit first, excluding shadows.
    pub ordinary: Vec<u8>,
    /// Same packing, including shadows.
    pub blipped: Vec<u8>,
}

impl SpriteOpacity {
    pub fn validate(&self) -> Result<()> {
        let pixels = usize::from(self.width) * usize::from(self.height);
        let bytes = pixels.div_ceil(8);
        ensure!(
            self.ordinary.len() == bytes && self.blipped.len() == bytes,
            "sprite {} opacity length disagrees with dimensions",
            self.bank_id
        );
        ensure!(
            self.ordinary
                .iter()
                .zip(&self.blipped)
                .all(|(ordinary, blipped)| ordinary & !blipped == 0),
            "sprite {} ordinary opacity exceeds blipped opacity",
            self.bank_id
        );
        if pixels % 8 != 0 {
            let unused = !((1u8 << (pixels % 8)) - 1);
            ensure!(
                self.ordinary[bytes - 1] & unused == 0 && self.blipped[bytes - 1] & unused == 0,
                "sprite {} opacity has nonzero padding",
                self.bank_id
            );
        }
        Ok(())
    }

    pub fn is_opaque(&self, x: u16, y: u16, blue_pixels_are_in: bool) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let index = usize::from(y) * usize::from(self.width) + usize::from(x);
        let bits = if blue_pixels_are_in {
            &self.blipped
        } else {
            &self.ordinary
        };
        bits[index / 8] & (1 << (index % 8)) != 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct OpacityBatch {
    pub sprites: Vec<SpriteOpacity>,
}

impl OpacityBatch {
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        for sprite in &self.sprites {
            sprite.validate()?;
            ensure!(
                ids.insert(sprite.bank_id),
                "duplicate opacity sprite {}",
                sprite.bank_id
            );
        }
        Ok(())
    }
}

pub fn encode(batch: &OpacityBatch) -> Result<Vec<u8>> {
    batch.validate()?;
    Ok(bitcode::encode(batch))
}

pub fn decode(bytes: &[u8]) -> Result<OpacityBatch> {
    let batch: OpacityBatch = bitcode::decode(bytes)?;
    batch.validate()?;
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_padding_subset_and_roundtrip() {
        let mut sprite = SpriteOpacity {
            bank_id: 7,
            width: 4,
            height: 1,
            dictionary_index: 0,
            ordinary: vec![4],
            blipped: vec![6],
        };
        assert!(!sprite.is_opaque(1, 0, false));
        assert!(sprite.is_opaque(1, 0, true));
        let batch = OpacityBatch {
            sprites: vec![sprite.clone()],
        };
        assert_eq!(
            decode(&encode(&batch).unwrap()).unwrap().sprites,
            batch.sprites
        );
        sprite.ordinary[0] = 1;
        assert!(sprite.validate().is_err());
        sprite.ordinary[0] = 4;
        sprite.blipped[0] |= 128;
        assert!(sprite.validate().is_err());
    }
}
