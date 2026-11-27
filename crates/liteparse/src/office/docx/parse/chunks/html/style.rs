//! Supported CSS cascade, computed independently from the host Word style hierarchy.
use super::dom::Handle;
use simplecss::{AttributeOperator, Element, PseudoClass};
use std::collections::{BTreeMap, BTreeSet};

/// Per-import work units cover rule dispatch and tree steps, plus bytes examined
/// by tag/attribute comparisons. Cloned selector cursors share exhaustion state.
struct SelectorWork {
    remaining: std::cell::Cell<usize>,
    exhausted: std::cell::Cell<bool>,
}

impl SelectorWork {
    fn charge(&self, amount: usize) -> bool {
        if self.exhausted.get() || amount > self.remaining.get() {
            self.exhausted.set(true);
            self.remaining.set(0);
            return false;
        }
        self.remaining.set(self.remaining.get() - amount);
        true
    }
}

#[derive(Clone)]
struct CssNode {
    node: Handle,
    work: std::rc::Rc<SelectorWork>,
}

impl CssNode {
    fn related(&self, node: Handle) -> Self {
        Self {
            node,
            work: self.work.clone(),
        }
    }
}

impl Element for CssNode {
    fn parent_element(&self) -> Option<Self> {
        if !self.work.charge(1) {
            return None;
        }
        self.node
            .parent
            .borrow()
            .upgrade()
            .filter(is_element)
            .map(|node| self.related(node))
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        if !self.work.charge(1) {
            return None;
        }
        self.node
            .previous_element
            .borrow()
            .upgrade()
            .map(|node| self.related(node))
    }
    fn has_local_name(&self, name: &str) -> bool {
        self.work.charge(1 + name.len() + self.node.tag().len())
            && self.node.tag().eq_ignore_ascii_case(name)
    }
    fn attribute_matches(&self, name: &str, op: AttributeOperator<'_>) -> bool {
        for attr in self.node.attrs.borrow().iter() {
            if !self.work.charge(1 + name.len() + attr.name.local.len()) {
                return false;
            }
            if attr.name.ns.is_empty() && &*attr.name.local == name {
                let operand = match op {
                    AttributeOperator::Exists => 0,
                    AttributeOperator::Matches(value)
                    | AttributeOperator::Contains(value)
                    | AttributeOperator::StartsWith(value) => value.len(),
                };
                return self.work.charge(1 + attr.value.len() + operand) && op.matches(&attr.value);
            }
        }
        self.work.charge(1);
        false
    }
    fn pseudo_class_matches(&self, class: PseudoClass<'_>) -> bool {
        if !self.work.charge(1) {
            return false;
        }
        match class {
            PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            PseudoClass::Link => {
                self.has_local_name("a")
                    && self.attribute_matches("href", AttributeOperator::Exists)
            }
            PseudoClass::Lang(language) => self.node.attr("lang").is_some_and(|value| {
                self.work.charge(1 + value.len() + language.len())
                    && value.eq_ignore_ascii_case(language)
            }),
            _ => false,
        }
    }
}

fn is_element(node: &Handle) -> bool {
    !node.tag().starts_with('#')
}

#[derive(Clone, Default, Debug)]
pub(super) struct Computed(pub BTreeMap<String, String>);

impl Computed {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
    pub fn inherited(&self) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(k, _)| {
                    matches!(
                        k.as_str(),
                        "font-family"
                            | "font-size"
                            | "font-weight"
                            | "font-style"
                            | "text-decoration"
                            | "color"
                            | "text-align"
                            | "white-space"
                            | "line-height"
                            | "direction"
                            | "list-style-type"
                    )
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
}

pub(super) struct Cascade<'a> {
    sheet: simplecss::StyleSheet<'a>,
    pub warnings: BTreeSet<String>,
    work: std::rc::Rc<SelectorWork>,
}

impl<'a> Cascade<'a> {
    pub fn new(css: &'a str) -> Self {
        let mut warnings = BTreeSet::new();
        let sheet = if !audited_sheet(css)
            || css.contains(['@', '\\'])
            || css.matches('{').count() > 4096
            || css.matches(',').count() > 4096
        {
            warnings.insert("unsupported stylesheet syntax or rule budget".into());
            simplecss::StyleSheet::new()
        } else {
            simplecss::StyleSheet::parse(css)
        };
        Self {
            sheet,
            warnings,
            work: std::rc::Rc::new(SelectorWork {
                remaining: std::cell::Cell::new(1_000_000),
                exhausted: std::cell::Cell::new(false),
            }),
        }
    }

    pub fn compute(&mut self, node: &Handle, parent: &Computed) -> Result<Computed, &'static str> {
        if !self.work.charge(self.sheet.rules.len()) {
            return Err("HTML selector work budget exceeded");
        }
        let mut result = parent.inherited();
        match node.tag() {
            "b" | "strong" | "th" => {
                result.0.insert("font-weight".into(), "bold".into());
            }
            "i" | "em" => {
                result.0.insert("font-style".into(), "italic".into());
            }
            "u" => {
                result
                    .0
                    .insert("text-decoration".into(), "underline".into());
            }
            "s" | "strike" | "del" => {
                result
                    .0
                    .insert("text-decoration".into(), "line-through".into());
            }
            "pre" => {
                result.0.insert("white-space".into(), "pre".into());
                result.0.insert("font-family".into(), "monospace".into());
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                result.0.insert("font-weight".into(), "bold".into());
                let size = match node.tag() {
                    "h1" => 24,
                    "h2" => 18,
                    "h3" => 14,
                    _ => 12,
                };
                result.0.insert("font-size".into(), format!("{size}pt"));
            }
            _ => {}
        }
        if let Some(align) = node.attr("align") {
            result.0.insert("text-align".into(), align);
        }
        let mut declarations = BTreeMap::<String, (bool, String)>::new();
        let element = CssNode {
            node: node.clone(),
            work: self.work.clone(),
        };
        for rule in &self.sheet.rules {
            // Descendant matching can revisit ancestors through many search paths.
            // Every cloned callback node shares the same monotonic work budget.
            let matched = rule.selector.matches(&element);
            if self.work.exhausted.get() {
                return Err("HTML selector work budget exceeded");
            }
            if matched {
                for declaration in &rule.declarations {
                    assign(
                        &mut declarations,
                        declaration.name,
                        declaration.value,
                        declaration.important,
                    );
                }
            }
        }
        if let Some(inline) = node.attr("style") {
            if !audited_declarations(&inline) || inline.contains(['@', '\\']) {
                self.warnings.insert("unsupported inline CSS syntax".into());
            } else {
                for declaration in simplecss::DeclarationTokenizer::from(inline.as_str()) {
                    assign(
                        &mut declarations,
                        declaration.name,
                        declaration.value,
                        declaration.important,
                    );
                }
            }
        }
        for (name, (_, value)) in declarations {
            if !supported(&name) {
                self.warnings
                    .insert(format!("unsupported CSS property {name}"));
                continue;
            }
            if value.eq_ignore_ascii_case("inherit") {
                if let Some(value) = parent.get(&name) {
                    result.0.insert(name, value.into());
                } else {
                    result.0.remove(&name);
                }
            } else if value.eq_ignore_ascii_case("unset") {
                if let Some(value) = parent.inherited().get(&name) {
                    result.0.insert(name, value.into());
                } else {
                    result.0.remove(&name);
                }
            } else if value.eq_ignore_ascii_case("initial") {
                result.0.remove(&name);
            } else if valid_value(&name, &value) {
                result.0.insert(name, value);
            } else {
                self.warnings
                    .insert(format!("unsupported CSS value for {name}"));
            }
        }
        let mut decorations = std::collections::BTreeSet::new();
        for value in [parent.get("text-decoration"), result.get("text-decoration")]
            .into_iter()
            .flatten()
        {
            decorations.extend(
                value
                    .split_whitespace()
                    .filter(|v| matches!(*v, "underline" | "line-through")),
            );
        }
        let decorations = decorations.into_iter().collect::<Vec<_>>().join(" ");
        if !decorations.is_empty() {
            result.0.insert("text-decoration".into(), decorations);
        }
        let parent_size = parent
            .get("font-size")
            .and_then(|v| length(v, 12.0))
            .unwrap_or(12.0);
        if let Some(value) = result.get("font-size").map(str::to_string) {
            if let Some(points) = length(&value, parent_size).filter(|n| *n > 0.0) {
                result.0.insert("font-size".into(), format!("{points}pt"));
            } else {
                self.warnings
                    .insert("unsupported CSS font-size value".into());
                result
                    .0
                    .insert("font-size".into(), format!("{parent_size}pt"));
            }
        }
        if let Some(value) = result.get("line-height").map(str::to_string) {
            let size = result
                .get("font-size")
                .and_then(|v| length(v, 12.0))
                .unwrap_or(parent_size);
            if value.parse::<f64>().is_err()
                && let Some(points) = length(&value, size)
            {
                result.0.insert("line-height".into(), format!("{points}pt"));
            }
        }
        Ok(result)
    }
}

fn assign(out: &mut BTreeMap<String, (bool, String)>, name: &str, value: &str, important: bool) {
    let name = name.to_ascii_lowercase();
    if name == "margin" {
        let values: Vec<_> = value.split_whitespace().collect();
        let wide = values.len() == 1
            && matches!(
                values[0].to_ascii_lowercase().as_str(),
                "inherit" | "initial" | "unset"
            );
        if wide
            || ((1..=4).contains(&values.len())
                && values.iter().all(|v| valid_value("margin-top", v)))
        {
            let top = values[0];
            let right = *values.get(1).unwrap_or(&top);
            let bottom = *values.get(2).unwrap_or(&top);
            let left = *values.get(3).unwrap_or(&right);
            // Expand at declaration order so each side retains cascade priority.
            for (side, value) in [
                ("top", top),
                ("right", right),
                ("bottom", bottom),
                ("left", left),
            ] {
                assign(out, &format!("margin-{side}"), value, important);
            }
            return;
        }
    }
    if out
        .get(&name)
        .is_some_and(|(prior, _)| *prior && !important)
    {
        return;
    }
    out.insert(name, (important, value.trim().to_string()));
}

fn supported(name: &str) -> bool {
    matches!(
        name,
        "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "color"
            | "background-color"
            | "text-decoration"
            | "text-align"
            | "white-space"
            | "line-height"
            | "direction"
            | "margin-top"
            | "margin-bottom"
            | "margin-left"
            | "margin-right"
            | "text-indent"
            | "width"
            | "height"
            | "padding"
            | "padding-top"
            | "padding-bottom"
            | "padding-left"
            | "padding-right"
            | "border"
            | "border-top"
            | "border-bottom"
            | "border-left"
            | "border-right"
            | "border-collapse"
            | "vertical-align"
            | "display"
            | "page-break-before"
            | "page-break-after"
            | "list-style-type"
    )
}

pub(super) fn length(value: &str, em: f64) -> Option<f64> {
    let value = value.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0.0);
    }
    let (number, scale) = if let Some(n) = value.strip_suffix("pt") {
        (n, 1.0)
    } else if let Some(n) = value.strip_suffix("px") {
        (n, 0.75)
    } else if let Some(n) = value.strip_suffix("in") {
        (n, 72.0)
    } else if let Some(n) = value.strip_suffix("cm") {
        (n, 72.0 / 2.54)
    } else if let Some(n) = value.strip_suffix("mm") {
        (n, 72.0 / 25.4)
    } else if let Some(n) = value.strip_suffix("em") {
        (n, em)
    } else {
        let n = value.strip_suffix('%')?;
        (n, em / 100.0)
    };
    number
        .parse::<f64>()
        .ok()
        .map(|n| n * scale)
        .filter(|n| n.is_finite() && n.abs() <= 100_000.0)
}

// This deliberately small syntax gate accounts for every rule and declaration before
// simplecss is called. Unsupported quoting, escapes, at-rules and selector forms are
// diagnosed together; parser-discarded rules cannot silently change the cascade.
fn audited_sheet(css: &str) -> bool {
    if css.contains(['@', '\\']) || css.contains("/*") {
        return false;
    }
    let mut rest = css.trim();
    let mut expected = 0;
    while !rest.is_empty() {
        let Some((selector, tail)) = rest.split_once('{') else {
            return false;
        };
        let Some((declarations, tail)) = tail.split_once('}') else {
            return false;
        };
        if selector.trim().is_empty()
            || !selector.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        ' ' | '\t' | '\r' | '\n' | '.' | '#' | '_' | '-' | '>' | '+' | '*' | ','
                    )
            })
            || !audited_declarations(declarations)
        {
            return false;
        }
        expected += selector.split(',').count();
        rest = tail.trim();
    }
    if expected > 4096 {
        return false;
    }
    simplecss::StyleSheet::parse(css).rules.len() == expected
}

fn audited_declarations(css: &str) -> bool {
    if css.contains(['{', '}', '@', '\\']) || css.contains("/*") {
        return false;
    }
    let expected = css.split(';').filter(|s| !s.trim().is_empty()).count();
    if expected > 4096 {
        return false;
    }
    css.split(';').filter(|s| !s.trim().is_empty()).all(|s| {
        let Some((key, value)) = s.split_once(':') else {
            return false;
        };
        !key.trim().is_empty()
            && key
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !value.trim().is_empty()
    }) && simplecss::DeclarationTokenizer::from(css).count() == expected
}

fn valid_value(name: &str, value: &str) -> bool {
    if value.contains('%')
        && !matches!(
            name,
            "font-size" | "line-height" | "width" | "height" | "color" | "background-color"
        )
    {
        return false;
    }
    if value.contains(['\\', '{', '}']) || value.contains("calc(") || value.contains("var(") {
        return false;
    }
    match name {
        "color" | "background-color" => {
            super::word::color(value).is_some() || value == "transparent"
        }
        "font-weight" => {
            matches!(value, "normal" | "bold")
                || value.parse::<u16>().is_ok_and(|v| (100..=900).contains(&v))
        }
        "font-style" => matches!(value, "normal" | "italic" | "oblique"),
        "text-decoration" => value
            .split_whitespace()
            .all(|v| matches!(v, "none" | "underline" | "line-through")),
        "text-align" => matches!(
            value,
            "left" | "right" | "center" | "justify" | "start" | "end"
        ),
        "direction" => matches!(value, "ltr" | "rtl"),
        "white-space" => matches!(value, "normal" | "pre" | "pre-wrap"),
        "display" => matches!(
            value,
            "none" | "block" | "inline" | "table" | "table-row" | "table-cell" | "list-item"
        ),
        "page-break-before" | "page-break-after" => matches!(value, "auto" | "always"),
        "vertical-align" => matches!(value, "top" | "middle" | "bottom" | "baseline"),
        "border-collapse" => matches!(value, "collapse" | "separate"),
        "list-style-type" => matches!(value, "disc" | "decimal" | "none"),
        "font-family" => !value.is_empty(),
        "line-height" => {
            value == "normal"
                || length(value, 12.0).is_some()
                || value.parse::<f64>().is_ok_and(|v| v.is_finite() && v > 0.0)
        }
        "padding" => {
            value.split_whitespace().count() <= 4
                && value.split_whitespace().all(|v| length(v, 12.0).is_some())
        }
        "border" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
            super::word::border(value).is_some()
        }
        _ => length(value, 12.0).is_some(),
    }
}
