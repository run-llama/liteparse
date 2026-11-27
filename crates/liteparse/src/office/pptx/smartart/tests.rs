use super::build::{self, Tree};
use super::data::{DataModel, PtType};
use super::layout::{Engine, Measure, Rect};
use super::*;

const NS: &str = r#"xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

/// A data model with `n` top-level nodes "A", "B", ... under the doc point.
fn data(n: usize) -> DataModel {
    let mut pts = String::from(r#"<dgm:pt modelId="doc" type="doc"/>"#);
    let mut cxns = String::new();
    for i in 0..n {
        let name = (b'A' + i as u8) as char;
        pts.push_str(&format!(
            r#"<dgm:pt modelId="n{i}"><dgm:t><a:bodyPr/><a:p><a:r><a:t>{name}</a:t></a:r></a:p></dgm:t></dgm:pt>
               <dgm:pt modelId="p{i}" type="parTrans"/><dgm:pt modelId="s{i}" type="sibTrans"/>"#
        ));
        cxns.push_str(&format!(
            r#"<dgm:cxn modelId="c{i}" srcId="doc" destId="n{i}" srcOrd="{i}" parTransId="p{i}" sibTransId="s{i}"/>"#
        ));
    }
    let xml = format!(
        r#"<dgm:dataModel {NS}><dgm:ptLst>{pts}</dgm:ptLst><dgm:cxnLst>{cxns}</dgm:cxnLst></dgm:dataModel>"#
    );
    DataModel::parse(&crate::office::pptx::xml::parse(&xml).unwrap()).unwrap()
}

fn tree(def: &str, dm: &DataModel) -> Tree {
    let xml = format!(r#"<dgm:layoutDef {NS}>{def}</dgm:layoutDef>"#);
    build::build(&crate::office::pptx::xml::parse(&xml).unwrap(), dm).unwrap()
}

/// Text always fits in one line of 1.2 × the font size.
struct OneLine;

impl Measure for OneLine {
    fn text_fit(&mut self, _: usize, fs: f64, _: f64, h: f64, m: [f64; 4]) -> (f64, f64) {
        (fs * 1.2, h * PT_PER_MM - m[1] - m[3])
    }
}

fn rects(e: &Engine, name: &str) -> Vec<Rect> {
    (0..e.tree.nodes.len())
        .filter(|&i| e.tree.nodes[i].name == name)
        .filter_map(|i| e.rect[i])
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn transitions_sit_between_nodes_and_the_last_is_hidden() {
    let dm = data(3);
    let kids: Vec<PtType> = dm.children[dm.root].iter().map(|&c| dm.pts[c].ty).collect();
    assert_eq!(kids.len(), 9);
    assert_eq!(kids[0], PtType::ParTrans);
    assert_eq!(kids[1], PtType::Node);
    assert_eq!(kids[2], PtType::SibTrans);
    let def = r#"<dgm:layoutNode name="root"><dgm:forEach axis="ch" ptType="node">
        <dgm:layoutNode name="node"/>
        <dgm:forEach axis="followSib" ptType="sibTrans" cnt="1"><dgm:layoutNode name="arrow"/></dgm:forEach>
        </dgm:forEach></dgm:layoutNode>"#;
    let t = tree(def, &dm);
    let count = |n: &str| t.nodes.iter().filter(|x| x.name == n).count();
    assert_eq!((count("node"), count("arrow")), (3, 2));
}

#[test]
fn choose_follows_the_child_count() {
    let def = r#"<dgm:layoutNode name="root"><dgm:choose>
        <dgm:if axis="ch" ptType="node" func="cnt" op="gte" val="3"><dgm:alg type="snake"/></dgm:if>
        <dgm:else><dgm:alg type="lin"/></dgm:else></dgm:choose></dgm:layoutNode>"#;
    assert_eq!(tree(def, &data(2)).nodes[0].alg.kind, "lin");
    assert_eq!(tree(def, &data(3)).nodes[0].alg.kind, "snake");
}

/// A process-like row: boxes as wide as the frame, arrows 0.4 of a box,
/// squeezed along the row; each box's height follows its own width.
#[test]
fn linear_row_shares_the_width() {
    let dm = data(3);
    let def = r#"<dgm:layoutNode name="root"><dgm:alg type="lin"/>
        <dgm:constrLst>
          <dgm:constr type="w" for="ch" ptType="node" refType="w"/>
          <dgm:constr type="w" for="ch" ptType="sibTrans" refType="w" refFor="ch" refPtType="node" fact="0.4"/>
        </dgm:constrLst>
        <dgm:forEach axis="ch" ptType="node">
          <dgm:layoutNode name="node"><dgm:alg type="tx"/><dgm:shape type="rect"/><dgm:presOf axis="self"/>
            <dgm:constrLst><dgm:constr type="h" refType="w" fact="0.6"/></dgm:constrLst></dgm:layoutNode>
          <dgm:forEach axis="followSib" ptType="sibTrans" cnt="1"><dgm:layoutNode name="gap"><dgm:alg type="sp"/></dgm:layoutNode></dgm:forEach>
        </dgm:forEach></dgm:layoutNode>"#;
    let t = tree(def, &dm);
    let mut e = Engine::new(&t, &dm);
    e.run(380.0, 200.0, &mut OneLine);
    let boxes = rects(&e, "node");
    // 3 boxes + 2 gaps of 0.4 box = 3.8 boxes.
    assert_eq!(boxes.len(), 3);
    assert!(close(boxes[0].w, 100.0), "{:?}", boxes[0]);
    assert!(close(boxes[0].h, 60.0), "{:?}", boxes[0]);
    assert!(close(boxes[1].x, 140.0) && close(boxes[2].x + boxes[2].w, 380.0));
    assert!(
        close(boxes[0].y, 70.0),
        "centred vertically: {:?}",
        boxes[0]
    );
}

#[test]
fn cycle_puts_the_first_node_on_top() {
    let dm = data(4);
    let def = r#"<dgm:layoutNode name="root"><dgm:alg type="cycle"/>
        <dgm:constrLst><dgm:constr type="w" for="ch" ptType="node" val="10"/>
          <dgm:constr type="h" for="ch" ptType="node" val="10"/></dgm:constrLst>
        <dgm:forEach axis="ch" ptType="node"><dgm:layoutNode name="node"><dgm:shape type="ellipse"/></dgm:layoutNode></dgm:forEach>
        </dgm:layoutNode>"#;
    let t = tree(def, &dm);
    let mut e = Engine::new(&t, &dm);
    e.run(100.0, 100.0, &mut OneLine);
    let n = rects(&e, "node");
    let (c0, c1, c2) = (
        (n[0].cx(), n[0].cy()),
        (n[1].cx(), n[1].cy()),
        (n[2].cx(), n[2].cy()),
    );
    assert!(close(c0.0, 50.0) && c0.1 < 50.0, "{c0:?}");
    assert!(c1.0 > 50.0 && close(c1.1, 50.0), "clockwise: {c1:?}");
    assert!(close(c2.0, 50.0) && c2.1 > 50.0, "{c2:?}");
}

#[test]
fn a_bare_declaration_does_not_replace_an_ancestor_value() {
    let dm = data(1);
    let def = r#"<dgm:layoutNode name="root"><dgm:alg type="composite"/>
        <dgm:constrLst><dgm:constr type="h" for="ch" forName="box" val="30"/>
          <dgm:constr type="w" for="ch" forName="box" val="40"/></dgm:constrLst>
        <dgm:layoutNode name="box"><dgm:constrLst><dgm:constr type="h"/></dgm:constrLst></dgm:layoutNode>
        </dgm:layoutNode>"#;
    let t = tree(def, &dm);
    let mut e = Engine::new(&t, &dm);
    e.run(100.0, 100.0, &mut OneLine);
    let b = rects(&e, "box")[0];
    assert!(close(b.h, 30.0) && close(b.w, 40.0), "{b:?}");
}

#[test]
fn composite_places_from_edges_and_offsets() {
    let dm = data(1);
    let def = r#"<dgm:layoutNode name="root"><dgm:alg type="composite"/>
        <dgm:constrLst>
          <dgm:constr type="l" for="ch" forName="a" refType="w" fact="0.1"/>
          <dgm:constr type="w" for="ch" forName="a" refType="w" fact="0.3"/>
          <dgm:constr type="l" for="ch" forName="b" refType="r" refFor="ch" refForName="a"/>
          <dgm:constr type="r" for="ch" forName="b" refType="w"/>
          <dgm:constr type="rOff" for="ch" forName="b" val="-5"/>
        </dgm:constrLst>
        <dgm:layoutNode name="a"/><dgm:layoutNode name="b"/></dgm:layoutNode>"#;
    let t = tree(def, &dm);
    let mut e = Engine::new(&t, &dm);
    e.run(100.0, 50.0, &mut OneLine);
    let b = rects(&e, "b")[0];
    assert!(close(b.x, 40.0) && close(b.w, 55.0), "{b:?}");
}
