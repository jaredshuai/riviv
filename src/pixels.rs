//! Pixel-buffer math (pure logic, unit-tested).
//!
//! M2 seam: alpha compositing over the window background (#3) and mipmap
//! generation math (#9) land here beside the BGRA conversion.

/// image crate yields RGBA rows (top-down); GDI 32bpp DIBs want BGRA.
pub(crate) fn rgba8_to_bgra_in_place(buf: &mut [u8]) {
    let (pixels, tail) = buf.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "RGBA8 buffer length must be a multiple of 4"
    );
    for px in pixels {
        px.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_conversion_swaps_r_and_b_in_place() {
        let mut pixels = vec![10, 20, 30, 255, 1, 2, 3, 128];
        rgba8_to_bgra_in_place(&mut pixels);
        assert_eq!(pixels, vec![30, 20, 10, 255, 3, 2, 1, 128]);
    }
}
