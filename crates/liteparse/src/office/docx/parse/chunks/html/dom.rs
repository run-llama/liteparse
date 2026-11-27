//! Bounded HTML5 tree construction; imported nodes remain independent of Word XML.
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, ExpandedName, QualName};

pub(super) type Handle = Rc<Node>;

#[derive(Debug)]
pub(super) struct Node {
    pub name: QualName,
    pub attrs: RefCell<Vec<Attribute>>,
    pub text: RefCell<String>,
    pub children: RefCell<Vec<Handle>>,
    pub parent: RefCell<Weak<Node>>,
    pub previous_element: RefCell<Weak<Node>>,
    template: RefCell<Option<Handle>>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .borrow()
            .iter()
            .find(|a| a.name.ns.is_empty() && &*a.name.local == name)
            .map(|a| a.value.to_string())
    }

    pub fn tag(&self) -> &str {
        self.name.local.as_ref()
    }
}

struct Sink {
    root: Handle,
    nodes: Rc<Cell<usize>>,
    limit: usize,
    exceeded: Rc<Cell<bool>>,
}

fn node(name: QualName, attrs: Vec<Attribute>, text: String) -> Handle {
    Rc::new(Node {
        name,
        attrs: RefCell::new(attrs),
        text: RefCell::new(text),
        children: RefCell::new(Vec::new()),
        parent: RefCell::new(Weak::new()),
        previous_element: RefCell::new(Weak::new()),
        template: RefCell::new(None),
    })
}

fn special(name: &str) -> QualName {
    QualName::new(None, "".into(), name.into())
}

impl Sink {
    fn allocate(&self, name: QualName, attrs: Vec<Attribute>, text: String) -> Handle {
        let count = self.nodes.get().saturating_add(1);
        self.nodes.set(count);
        if count > self.limit {
            self.exceeded.set(true);
        }
        node(name, attrs, text)
    }

    fn insert(&self, parent: &Handle, index: usize, child: NodeOrText<Handle>) {
        let mut ancestor = Some(parent.clone());
        let mut depth = 0;
        while let Some(p) = ancestor {
            depth += 1;
            if depth > 256 {
                self.exceeded.set(true);
                return;
            }
            ancestor = p.parent.borrow().upgrade();
        }
        let child = match child {
            NodeOrText::AppendText(text) => {
                if index > 0 {
                    let previous = parent.children.borrow().get(index - 1).cloned();
                    if let Some(previous) = previous.filter(|n| n.tag() == "#text") {
                        previous.text.borrow_mut().push_str(&text);
                        return;
                    }
                }
                self.allocate(special("#text"), Vec::new(), text.to_string())
            }
            NodeOrText::AppendNode(child) => child,
        };
        self.remove_from_parent(&child);
        *child.parent.borrow_mut() = Rc::downgrade(parent);
        let mut children = parent.children.borrow_mut();
        let index = index.min(children.len());
        children.insert(index, child);
    }
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Handle;
    type ElemName<'a> = ExpandedName<'a>;
    fn finish(self) -> Handle {
        self.root
    }
    fn parse_error(&self, _: Cow<'static, str>) {}
    fn get_document(&self) -> Handle {
        self.root.clone()
    }
    fn elem_name<'a>(&'a self, node: &'a Handle) -> ExpandedName<'a> {
        node.name.expanded()
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let result = self.allocate(name, attrs, String::new());
        if flags.template {
            *result.template.borrow_mut() =
                Some(self.allocate(special("#fragment"), Vec::new(), String::new()));
        }
        result
    }
    fn create_comment(&self, _: StrTendril) -> Handle {
        self.allocate(special("#comment"), Vec::new(), String::new())
    }
    fn create_pi(&self, _: StrTendril, _: StrTendril) -> Handle {
        self.create_comment(StrTendril::new())
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        let index = parent.children.borrow().len();
        self.insert(parent, index, child);
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        previous: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if element.parent.borrow().upgrade().is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(previous, child);
        }
    }
    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}
    fn get_template_contents(&self, target: &Handle) -> Handle {
        target.template.borrow().as_ref().unwrap().clone()
    }
    fn same_node(&self, a: &Handle, b: &Handle) -> bool {
        Rc::ptr_eq(a, b)
    }
    fn set_quirks_mode(&self, _: QuirksMode) {}
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let parent = sibling.parent.borrow().upgrade();
        if let Some(parent) = parent {
            let index = parent
                .children
                .borrow()
                .iter()
                .position(|n| Rc::ptr_eq(n, sibling));
            if let Some(index) = index {
                self.insert(&parent, index, child);
            }
        }
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut existing = target.attrs.borrow_mut();
        for attr in attrs {
            if !existing.iter().any(|a| a.name == attr.name) {
                existing.push(attr);
            }
        }
    }
    fn remove_from_parent(&self, target: &Handle) {
        let parent = std::mem::take(&mut *target.parent.borrow_mut()).upgrade();
        if let Some(parent) = parent {
            parent
                .children
                .borrow_mut()
                .retain(|n| !Rc::ptr_eq(n, target));
        }
    }
    fn reparent_children(&self, source: &Handle, target: &Handle) {
        let children = std::mem::take(&mut *source.children.borrow_mut());
        for child in children {
            *child.parent.borrow_mut() = Weak::new();
            self.append(target, NodeOrText::AppendNode(child));
        }
    }
}

/// Small input batches bound tree-builder work after the sink reaches its node limit.
/// A single token can hold input-sized text/attributes; the enclosing byte budget bounds it.
pub(super) fn parse(text: &str, limit: usize) -> Result<(Handle, usize), &'static str> {
    let nodes = Rc::new(Cell::new(1));
    let exceeded = Rc::new(Cell::new(false));
    let sink = Sink {
        root: node(special("#document"), Vec::new(), String::new()),
        nodes: nodes.clone(),
        limit,
        exceeded: exceeded.clone(),
    };
    let mut parser = html5ever::parse_document(sink, Default::default());
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + 64).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        parser.process(StrTendril::from_slice(&text[start..end]));
        if exceeded.get() {
            return Err("HTML tree budget exceeded");
        }
        start = end;
    }
    let result = parser.finish();
    if exceeded.get() {
        return Err("HTML tree budget exceeded");
    }
    let mut pending = vec![result.clone()];
    while let Some(parent) = pending.pop() {
        let mut previous = Weak::new();
        for child in parent.children.borrow().iter() {
            *child.previous_element.borrow_mut() = previous.clone();
            if !child.tag().starts_with('#') {
                previous = Rc::downgrade(child);
            }
            pending.push(child.clone());
        }
    }
    Ok((result, nodes.get()))
}

pub(super) fn empty_row() -> Handle {
    node(
        QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "tr".into()),
        Vec::new(),
        String::new(),
    )
}
