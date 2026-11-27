use super::*;

#[test]
fn highlight_palette_values_preserve_numbered_and_bullet_labels_across_property_layers() {
    for bullet in [false, true] {
        for value in 0..=0x10 {
            for layer in 0..3 {
                let mut f = Fixture::new(&[if bullet { 2 } else { 1 }; 2]);
                let bytes = vec![0x0c, 0x2a, value];
                match layer {
                    0 => f.styles.styles[0]
                        .as_mut()
                        .unwrap()
                        .properties
                        .push(StyleProperties {
                            kind: "character",
                            bytes,
                        }),
                    1 => {
                        f.formatting.character_spans[0].properties =
                            decode_character_modifiers(&bytes).unwrap()
                    }
                    _ => f.definitions.lists[usize::from(bullet)].levels[0]
                        .raw_chpx
                        .extend(bytes),
                }
                let out = f.run().unwrap();
                assert_eq!(
                    out.stories[0].text,
                    if bullet {
                        "•\tItem\n•\tItem\n"
                    } else {
                        "1.\tItem\n2.\tItem\n"
                    }
                );
                partition(&out.stories[0], 10);
            }
        }
    }
}

#[test]
fn malformed_highlight_palette_blocks_its_family_without_hiding_other_labels() {
    for operand in [
        vec![],
        vec![0x11],
        vec![0x7f],
        vec![0x82],
        vec![0xff],
        vec![0, 1],
    ] {
        assert!(!visible_modifier(&PropertyModifier {
            sprm: 0x2a0c,
            operand
        }));
    }
    let mut f = Fixture::new(&[1, 2, 1]);
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x0c, 0x2a, 0x11];
    let out = f.run().unwrap();
    let labels: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(labels, [5]);
    partition(&out.stories[0], 15);
}

#[test]
fn highlight_palette_values_retain_visibility_and_font_guards() {
    for sprm in [0x0800, 0x0801, 0x0802, 0x083c, 0x085a, 0x0818] {
        let mut f = Fixture::new(&[1, 1]);
        f.formatting.character_spans[0].properties = vec![
            PropertyModifier {
                sprm: 0x2a0c,
                operand: vec![1],
            },
            PropertyModifier {
                sprm,
                operand: vec![1],
            },
            PropertyModifier {
                sprm: 0x2a0c,
                operand: vec![0],
            },
        ];
        assert!(
            !f.run().unwrap().stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_list_label")
        );
    }
    let mut f = Fixture::new(&[2, 2]);
    f.definitions.lists[1].levels[0]
        .raw_chpx
        .extend([0x0c, 0x2a, 7]);
    f.fonts.fonts[0].name = "Unresolved synthetic font".into();
    assert!(
        !f.run().unwrap().stories[0]
            .segments
            .iter()
            .any(|s| s.kind == "generated_list_label")
    );
}
