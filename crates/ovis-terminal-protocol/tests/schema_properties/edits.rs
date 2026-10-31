//! Edits that narrow or widen a generated schema tree at one node.

use super::tree::Node;

/// Narrows `node`: the result accepts strictly fewer input values.
#[cfg(test)]
pub fn narrow(node: &Node) -> Node {
    match node.clone() {
        scalar @ (Node::Str { .. } | Node::Int { .. }) => narrow_scalar(scalar),
        Node::Obj {
            props,
            closed: false,
        } => Node::Obj {
            props,
            closed: true,
        },
        Node::Obj {
            mut props,
            closed: true,
        } => {
            match props.iter_mut().find(|(_, required)| !*required) {
                Some(optional) => optional.1 = true,
                None => props.push((
                    Node::Str {
                        pattern: None,
                        max_len: None,
                    },
                    true,
                )),
            }
            Node::Obj {
                props,
                closed: true,
            }
        }
        Node::Arr {
            items,
            max_items: Some(n),
        } if n > 0 => Node::Arr {
            items,
            max_items: Some(n.saturating_sub(1)),
        },
        Node::Arr {
            items,
            max_items: Some(n),
        } => Node::Arr {
            items: Box::new(narrow(&items)),
            max_items: Some(n),
        },
        Node::Arr {
            items,
            max_items: None,
        } => Node::Arr {
            items,
            max_items: Some(3),
        },
        Node::Nullable(child) => Node::Nullable(Box::new(narrow_values(&child))),
        Node::Ref(child) => Node::Ref(Box::new(narrow(&child))),
        Node::Within(child) => Node::Within(Box::new(narrow(&child))),
        Node::Guarded { child, .. } if matches!(*child, Node::Str { .. }) => Node::Guarded {
            child,
            pattern: Some("^b".to_owned()),
        },
        Node::Guarded { child, pattern } => Node::Guarded {
            child: Box::new(narrow_values(&child)),
            pattern,
        },
    }
}

/// Narrows the non-null values of `node`, keeping every null branch: an
/// ancestor may accept null as well, so dropping one would narrow nothing.
#[cfg(test)]
fn narrow_values(node: &Node) -> Node {
    match node {
        Node::Nullable(child) => Node::Nullable(Box::new(narrow_values(child))),
        Node::Ref(child) => Node::Ref(Box::new(narrow_values(child))),
        other => narrow(other),
    }
}

/// Widens `node`: the result accepts every value it accepted before.
#[cfg(test)]
pub fn widen(node: &Node) -> Node {
    match node.clone() {
        Node::Str {
            pattern: Some(_),
            max_len,
        } => Node::Str {
            pattern: None,
            max_len,
        },
        Node::Str {
            pattern: None,
            max_len: Some(n),
        } => Node::Str {
            pattern: None,
            max_len: Some(n.saturating_add(1)),
        },
        Node::Int {
            max,
            values: Some(mut values),
        } => {
            values.push(values.iter().max().map_or(0, |m| m.saturating_add(1)));
            Node::Int {
                max,
                values: Some(values),
            }
        }
        Node::Int {
            max: Some(m),
            values: None,
        } => Node::Int {
            max: Some(m.saturating_add(1)),
            values: None,
        },
        Node::Obj { mut props, closed } if props.iter().any(|(_, required)| *required) => {
            for prop in &mut props {
                prop.1 = false;
            }
            Node::Obj { props, closed }
        }
        Node::Obj {
            props,
            closed: true,
        } => Node::Obj {
            props,
            closed: false,
        },
        Node::Arr {
            items,
            max_items: Some(n),
        } => Node::Arr {
            items,
            max_items: Some(n.saturating_add(1)),
        },
        Node::Nullable(child) => Node::Nullable(Box::new(widen(&child))),
        Node::Ref(child) => Node::Ref(Box::new(widen(&child))),
        Node::Within(child) => Node::Within(Box::new(widen(&child))),
        Node::Guarded {
            child,
            pattern: Some(_),
        } => Node::Guarded {
            child,
            pattern: None,
        },
        Node::Guarded {
            child,
            pattern: None,
        } => Node::Guarded {
            child: Box::new(widen(&child)),
            pattern: None,
        },
        other => Node::Nullable(Box::new(other)),
    }
}

/// Narrows a string or integer node.
#[cfg(test)]
fn narrow_scalar(node: Node) -> Node {
    match node {
        Node::Str {
            pattern,
            max_len: Some(n),
        } if n > 0 => Node::Str {
            pattern,
            max_len: Some(n.saturating_sub(1)),
        },
        Node::Str {
            max_len: Some(n), ..
        } => Node::Str {
            pattern: Some("^b".to_owned()),
            max_len: Some(n),
        },
        Node::Str {
            pattern,
            max_len: None,
        } => Node::Str {
            pattern,
            max_len: Some(5),
        },
        Node::Int {
            max,
            values: Some(mut values),
        } if values.len() > 1 => {
            values.pop();
            Node::Int {
                max,
                values: Some(values),
            }
        }
        Node::Int {
            max,
            values: Some(values),
        } => Node::Int {
            max,
            values: Some(values.iter().map(|v| v.saturating_add(1000)).collect()),
        },
        Node::Int {
            max: Some(m),
            values: None,
        } => Node::Int {
            max: Some(m.saturating_sub(1)),
            values: None,
        },
        Node::Int {
            max: None,
            values: None,
        } => Node::Int {
            max: Some(10),
            values: None,
        },
        other => other,
    }
}

/// Number of nodes in preorder.
#[cfg(test)]
pub fn count(node: &Node) -> usize {
    1_usize.saturating_add(match node {
        Node::Str { .. } | Node::Int { .. } => 0,
        Node::Obj { props, .. } => props.iter().map(|(child, _)| count(child)).sum(),
        Node::Arr { items: child, .. }
        | Node::Nullable(child)
        | Node::Ref(child)
        | Node::Within(child)
        | Node::Guarded { child, .. } => count(child),
    })
}

/// Applies `change` to the node at preorder position `target`.
#[cfg(test)]
pub fn edit_at(node: &Node, target: &mut Option<usize>, change: fn(&Node) -> Node) -> Node {
    match target.take() {
        Some(0) => return change(node),
        Some(n) => *target = Some(n.saturating_sub(1)),
        None => {}
    }
    match node {
        Node::Str { .. } | Node::Int { .. } => node.clone(),
        Node::Obj { props, closed } => Node::Obj {
            props: props
                .iter()
                .map(|(child, req)| (edit_at(child, target, change), *req))
                .collect(),
            closed: *closed,
        },
        Node::Arr { items, max_items } => Node::Arr {
            items: Box::new(edit_at(items, target, change)),
            max_items: *max_items,
        },
        Node::Nullable(child) => Node::Nullable(Box::new(edit_at(child, target, change))),
        Node::Ref(child) => Node::Ref(Box::new(edit_at(child, target, change))),
        Node::Within(child) => Node::Within(Box::new(edit_at(child, target, change))),
        Node::Guarded { child, pattern } => Node::Guarded {
            child: Box::new(edit_at(child, target, change)),
            pattern: pattern.clone(),
        },
    }
}
