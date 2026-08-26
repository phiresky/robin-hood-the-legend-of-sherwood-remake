//! Native-bitcode adapters for local types whose storage uses third-party
//! types that do not implement bitcode's native traits.
//!
//! These adapters deliberately do not use bitcode's Serde backend. Each local
//! type defines an explicit native wire type and lossless conversions to and
//! from it.

use std::marker::PhantomData;
use std::num::NonZeroUsize;

use bitcode::__private::{Buffer, Decoder, Encoder, Result, View};

#[doc(hidden)]
pub trait NativeBitcode: Sized {
    type Wire: bitcode::Encode + bitcode::DecodeOwned;

    fn to_wire(&self) -> Self::Wire;
    fn from_wire(wire: Self::Wire) -> Self;
}

pub struct NativeEncoder<T: NativeBitcode> {
    wire: <<T as NativeBitcode>::Wire as bitcode::Encode>::Encoder,
    marker: PhantomData<fn() -> T>,
}

impl<T: NativeBitcode> Default for NativeEncoder<T> {
    fn default() -> Self {
        Self {
            wire: Default::default(),
            marker: PhantomData,
        }
    }
}

impl<T: NativeBitcode> Encoder<T> for NativeEncoder<T> {
    fn encode(&mut self, value: &T) {
        self.wire.encode(&value.to_wire());
    }
}

impl<T: NativeBitcode> Buffer for NativeEncoder<T> {
    fn collect_into(&mut self, out: &mut Vec<u8>) {
        self.wire.collect_into(out);
    }

    fn reserve(&mut self, additional: NonZeroUsize) {
        self.wire.reserve(additional);
    }
}

pub struct NativeDecoder<'de, T: NativeBitcode> {
    wire: <<T as NativeBitcode>::Wire as bitcode::Decode<'de>>::Decoder,
    marker: PhantomData<fn() -> T>,
}

impl<'de, T: NativeBitcode> Default for NativeDecoder<'de, T> {
    fn default() -> Self {
        Self {
            wire: Default::default(),
            marker: PhantomData,
        }
    }
}

impl<'de, T: NativeBitcode> View<'de> for NativeDecoder<'de, T> {
    fn populate(&mut self, input: &mut &'de [u8], length: usize) -> Result<()> {
        self.wire.populate(input, length)
    }
}

impl<'de, T: NativeBitcode> Decoder<'de, T> for NativeDecoder<'de, T> {
    fn decode(&mut self) -> T {
        T::from_wire(self.wire.decode())
    }
}

macro_rules! impl_native_bitcode {
    ($type:ty) => {
        impl bitcode::Encode for $type {
            type Encoder = crate::bitcode_adapters::NativeEncoder<Self>;
        }

        impl<'de> bitcode::Decode<'de> for $type {
            type Decoder = crate::bitcode_adapters::NativeDecoder<'de, Self>;
        }
    };
}

pub(crate) use impl_native_bitcode;

macro_rules! impl_native_bitcode_index {
    ($type:ty, $wire:ty) => {
        impl crate::bitcode_adapters::NativeBitcode for $type {
            type Wire = $wire;

            fn to_wire(&self) -> Self::Wire {
                self.get()
            }

            fn from_wire(wire: Self::Wire) -> Self {
                Self::new(wire).expect("native bitcode decoded a reserved index sentinel")
            }
        }

        crate::bitcode_adapters::impl_native_bitcode!($type);
    };
}

pub(crate) use impl_native_bitcode_index;

macro_rules! impl_native_bitcode_flags {
    ($type:ty, $wire:ty) => {
        impl crate::bitcode_adapters::NativeBitcode for $type {
            type Wire = $wire;

            fn to_wire(&self) -> Self::Wire {
                self.bits()
            }

            fn from_wire(wire: Self::Wire) -> Self {
                Self::from_bits_retain(wire)
            }
        }

        crate::bitcode_adapters::impl_native_bitcode!($type);
    };
}

pub(crate) use impl_native_bitcode_flags;

macro_rules! impl_native_bitcode_rect {
    ($type:ty) => {
        impl crate::bitcode_adapters::NativeBitcode for $type {
            type Wire = Option<([f32; 2], [f32; 2])>;

            fn to_wire(&self) -> Self::Wire {
                self.0
                    .map(|rect| ([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]))
            }

            fn from_wire(wire: Self::Wire) -> Self {
                Self(wire.map(|([min_x, min_y], [max_x, max_y])| {
                    geo::Rect::new(
                        geo::Coord { x: min_x, y: min_y },
                        geo::Coord { x: max_x, y: max_y },
                    )
                }))
            }
        }

        crate::bitcode_adapters::impl_native_bitcode!($type);
    };
}

pub(crate) use impl_native_bitcode_rect;
