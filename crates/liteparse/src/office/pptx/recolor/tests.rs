use super::*;

fn one_pixel(r: u8, g: u8, b: u8) -> Pixmap {
    let mut pm = Pixmap::new(1, 1).unwrap();
    pm.data_mut().copy_from_slice(&[r, g, b, 255]);
    pm
}

#[test]
fn duotone_maps_black_and_white_to_the_ramp_ends() {
    let red = Rgba {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let mut pm = one_pixel(0, 0, 0);
    apply(&mut pm, &[BlipEffect::Duotone(red, Rgba::WHITE)]);
    assert_eq!(pm.data(), &[255, 0, 0, 255]);
    let mut pm = one_pixel(255, 255, 255);
    apply(&mut pm, &[BlipEffect::Duotone(red, Rgba::WHITE)]);
    assert_eq!(pm.data(), &[255, 255, 255, 255]);
}

#[test]
fn clr_change_makes_the_color_transparent() {
    let mut pm = one_pixel(255, 255, 255);
    let clear = Rgba {
        a: 0.0,
        ..Rgba::WHITE
    };
    apply(
        &mut pm,
        &[BlipEffect::ClrChange {
            from: Rgba::WHITE,
            to: clear,
        }],
    );
    assert_eq!(pm.data()[3], 0);
}

#[test]
fn washout_lum_keeps_the_picture_visible() {
    let effect = [BlipEffect::Lum {
        bright: 0.7,
        contrast: -0.7,
    }];
    let mut black = one_pixel(0, 0, 0);
    apply(&mut black, &effect);
    let mut gray = one_pixel(128, 128, 128);
    apply(&mut gray, &effect);
    // Black lands near 0.78, mid-gray near 0.93: light but not white.
    assert!((190..210).contains(&black.data()[0]), "{:?}", black.data());
    assert!(black.data()[0] < gray.data()[0] && gray.data()[0] < 245);
}
