use super::*;

#[test]
fn frame_drain_continues_only_after_a_decoded_frame() {
    assert!(decoded_frame_available(Ok(()), "test"));
    for error in [
        ffmpeg_next::Error::Eof,
        ffmpeg_next::Error::Other {
            errno: ffmpeg_next::error::EAGAIN,
        },
        ffmpeg_next::Error::InvalidData,
        ffmpeg_next::Error::Other {
            errno: ffmpeg_next::error::EIO,
        },
    ] {
        assert!(!decoded_frame_available(Err(error), "test"));
    }
}

#[test]
fn rgba_scaling_reuses_storage_and_overwrites_previous_pixels() {
    use ffmpeg_next::{format::Pixel, frame::Video, software::scaling};
    let mut input = Video::new(Pixel::RGB24, 3, 2);
    let mut output = Video::empty();
    let mut scaler = scaling::Context::get(
        Pixel::RGB24,
        3,
        2,
        Pixel::RGBA,
        3,
        2,
        scaling::Flags::BILINEAR,
    )
    .unwrap();
    let mut output_pointer = None;
    for color in [[255, 0, 0], [0, 128, 255], [4, 5, 6]] {
        let stride = input.stride(0);
        input.data_mut(0).fill(0);
        for y in 0..2 {
            for x in 0..3 {
                input.data_mut(0)[y * stride + x * 3..y * stride + x * 3 + 3]
                    .copy_from_slice(&color);
            }
        }
        scaler.run(&input, &mut output).unwrap();
        let pointer = output.data(0).as_ptr();
        assert_eq!(*output_pointer.get_or_insert(pointer), pointer);
        assert_eq!((output.width(), output.height()), (3, 2));
        let stride = output.stride(0);
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(
                    &output.data(0)[y * stride + x * 4..y * stride + x * 4 + 4],
                    &[color[0], color[1], color[2], 255],
                );
            }
        }
    }
}
