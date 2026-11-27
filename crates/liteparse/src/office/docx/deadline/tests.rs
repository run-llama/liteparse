use super::*;
use crate::office::docx::testutil::{docx, pages, para};

#[test]
fn expired_budget_stops_layout() {
    let body: String = (0..50).map(|i| para(&format!("line {i}"))).collect();
    let bytes = docx(&body, None, None);
    start(Some(Duration::ZERO));
    let r = std::panic::catch_unwind(|| pages(bytes));
    start(None);
    let err = r.err().expect("layout should stop");
    assert!(expired(&err));
}

#[test]
fn no_budget_lays_out() {
    start(None);
    assert_eq!(pages(docx(&para("x"), None, None)).len(), 1);
}
