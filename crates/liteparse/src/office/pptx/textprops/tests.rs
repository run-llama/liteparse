use super::*;
use crate::office::pptx::xml::parse;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

#[test]
fn folds_list_styles_general_to_specific() {
    let master = parse(&format!(
        r#"<a:lstStyle {NS}><a:lvl1pPr marL="342900" indent="-342900" algn="l"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc>
           <a:buFont typeface="Arial"/><a:buChar char="•"/><a:defRPr sz="2800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr>
           <a:lvl2pPr marL="742950"><a:defRPr sz="2400"/></a:lvl2pPr></a:lstStyle>"#
    ))
    .unwrap();
    let shape = parse(&format!(r#"<a:lstStyle {NS}><a:lvl1pPr algn="ctr"><a:defRPr sz="2000" b="1"/></a:lvl1pPr></a:lstStyle>"#)).unwrap();
    let own = parse(&format!(r#"<a:pPr {NS}><a:buNone/></a:pPr>"#)).unwrap();
    let p = resolve_para(&[master.clone(), shape], 0, Some(&own));
    assert_eq!(p.align.as_deref(), Some("ctr"));
    assert_eq!(p.mar_l, Some(27.0));
    assert_eq!(p.indent, Some(-27.0));
    assert_eq!(p.line_spacing, Some(Spacing::Pct(0.9)));
    assert_eq!(p.bullet, Some(Bullet::None));
    assert_eq!(p.def_run.size, Some(20.0));
    assert_eq!(p.def_run.bold, Some(true));
    assert_eq!(p.def_run.latin.as_deref(), Some("+mn-lt"));
    let p2 = resolve_para(&[master], 1, None);
    assert_eq!(p2.def_run.size, Some(24.0));
}

#[test]
fn reads_autofit_scale() {
    let b = parse(&format!(r#"<a:bodyPr {NS} lIns="0" anchor="ctr"><a:normAutofit fontScale="62500" lnSpcReduction="20000"/></a:bodyPr>"#)).unwrap();
    let mut bp = BodyProps::default();
    bp.merge(&b);
    assert_eq!(bp.l_ins, Some(0.0));
    assert_eq!(bp.font_scale, Some(0.625));
    assert_eq!(bp.ln_spc_reduction, Some(0.2));
}

#[test]
fn uniform_list_style_sets_every_level() {
    let fill = solid_fill_xml(crate::office::pptx::color::Rgba {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 0.5,
    });
    assert!(fill.contains(r#"val="FF0000""#) && fill.contains(r#"val="50000""#));
    let ls = uniform_list_style(r#" b="1""#, &fill).unwrap();
    for l in 1..=9 {
        let d = ls.path(&[&format!("a:lvl{l}pPr"), "a:defRPr"]).unwrap();
        assert_eq!(d.attr("b"), Some("1"));
        assert!(d.child("a:solidFill").is_some());
    }
}
