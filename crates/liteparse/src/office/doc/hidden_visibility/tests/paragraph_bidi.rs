use super::*;

#[test]
fn direction_style_preserves_hidden_toggle_and_table_membership() {
    for rtl in [0, 1] {
        let mut styles = sheet();
        hidden(&mut styles, 0, 1);
        styles.styles[1].as_mut().unwrap().base_style = Some(0);
        styles.styles[0]
            .as_mut()
            .unwrap()
            .properties
            .push(StyleProperties {
                kind: "paragraph",
                bytes: vec![0, 0, 0x41, 0x24, rtl],
            });
        for table in [false, true] {
            let mut direct = vec![0x41, 0x24, 1, 0x41, 0x24, 0];
            if table {
                direct.extend([0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0]);
            }
            let result = paragraph::resolve(
                1,
                &direct,
                0,
                &[],
                13,
                &styles,
                &mut std::collections::BTreeMap::new(),
                &mut (8 * 1024 * 1024),
            )
            .unwrap();
            assert_eq!(result.style, Some(1));
            assert_eq!(result.non_table, !table);
            assert_eq!(result.table_depth, Some(u32::from(table)));
            assert_eq!(
                resolve_hidden_property(&styles, result.style.unwrap(), Some(false), &[])
                    .unwrap()
                    .hidden,
                Some(true)
            );
        }
    }
}

#[test]
fn invalid_direction_operand_cannot_establish_style_visibility() {
    for from_style in [false, true] {
        let mut styles = sheet();
        styles.styles[1].as_mut().unwrap().base_style = Some(0);
        let direct = if from_style {
            styles.styles[0]
                .as_mut()
                .unwrap()
                .properties
                .push(StyleProperties {
                    kind: "paragraph",
                    bytes: vec![0, 0, 0x41, 0x24, 2],
                });
            vec![]
        } else {
            vec![0x41, 0x24, 2, 0x41, 0x24, 0]
        };
        assert!(
            paragraph::resolve(
                1,
                &direct,
                0,
                &[],
                13,
                &styles,
                &mut std::collections::BTreeMap::new(),
                &mut (8 * 1024 * 1024)
            )
            .is_err()
        );
    }
}
