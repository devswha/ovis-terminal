//! Generated schema trees and their JSON Schema rendering.

use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// A generated input schema.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Node {
    /// A string with an optional pattern and length bound.
    Str {
        /// Pattern every value matches.
        pattern: Option<String>,
        /// Largest accepted length.
        max_len: Option<u32>,
    },
    /// An integer with an optional maximum and finite value set.
    Int {
        /// Largest accepted value.
        max: Option<i64>,
        /// Finite set of accepted values.
        values: Option<Vec<i64>>,
    },
    /// An object whose properties carry a required flag.
    Obj {
        /// Properties and whether each is required.
        props: Vec<(Self, bool)>,
        /// Whether unknown properties are rejected.
        closed: bool,
    },
    /// An array of one item schema with an optional item bound.
    Arr {
        /// Item schema.
        items: Box<Self>,
        /// Largest accepted item count.
        max_items: Option<u32>,
    },
    /// A value or null.
    Nullable(Box<Self>),
    /// A schema reached through a component reference.
    Ref(Box<Self>),
    /// A schema stated as the only member of an intersection nested in another.
    Within(Box<Self>),
    /// A value or null, with an optional pattern beside the union.
    Guarded {
        /// The non-null member.
        child: Box<Self>,
        /// Pattern every string value matches, stated beside `anyOf`.
        pattern: Option<String>,
    },
}

/// Strategy for schema trees up to four levels deep.
#[cfg(test)]
pub fn node() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        (
            prop::option::of(Just("^a".to_owned())),
            prop::option::of(1_u32..20)
        )
            .prop_map(|(pattern, max_len)| Node::Str { pattern, max_len }),
        (
            prop::option::of(0_i64..100),
            prop::option::of(prop::collection::btree_set(0_i64..50, 1..4))
        )
            .prop_map(|(max, values)| Node::Int {
                max,
                values: values.map(|set| set.into_iter().collect())
            }),
    ];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            (
                prop::collection::vec((inner.clone(), any::<bool>()), 1..3),
                any::<bool>()
            )
                .prop_map(|(props, closed)| Node::Obj { props, closed }),
            (inner.clone(), prop::option::of(1_u32..10)).prop_map(|(items, max_items)| Node::Arr {
                items: Box::new(items),
                max_items
            }),
            inner
                .clone()
                .prop_map(|child| Node::Nullable(Box::new(child))),
            inner.clone().prop_map(|child| Node::Ref(Box::new(child))),
            inner
                .clone()
                .prop_map(|child| Node::Within(Box::new(child))),
            (inner, prop::option::of(Just("^a".to_owned()))).prop_map(|(child, pattern)| {
                Node::Guarded {
                    child: Box::new(child),
                    pattern,
                }
            }),
        ]
    })
}

/// Renders nodes to JSON Schema, collecting `$ref` targets as definitions.
#[derive(Default)]
struct Renderer {
    definitions: Map<String, Value>,
}

impl Renderer {
    fn render(&mut self, node: &Node) -> Value {
        match node {
            Node::Str { pattern, max_len } => {
                json!({"type": "string", "pattern": pattern, "maxLength": max_len})
            }
            Node::Int { max, values } => json!({"type": "integer", "maximum": max, "enum": values}),
            Node::Obj { props, closed } => {
                let properties: Map<String, Value> = props
                    .iter()
                    .enumerate()
                    .map(|(i, (child, _))| (format!("p{i}"), self.render(child)))
                    .collect();
                let required: Vec<String> = props
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, req))| *req)
                    .map(|(i, _)| format!("p{i}"))
                    .collect();
                let mut object =
                    json!({"type": "object", "properties": properties, "required": required});
                if *closed {
                    object
                        .as_object_mut()
                        .map(|o| o.insert("additionalProperties".to_owned(), json!(false)));
                }
                object
            }
            Node::Arr { items, max_items } => {
                json!({"type": "array", "items": self.render(items), "maxItems": max_items})
            }
            Node::Nullable(child) => json!({"anyOf": [self.render(child), {"type": "null"}]}),
            Node::Within(child) => json!({"allOf": [{"allOf": [self.render(child)]}]}),
            Node::Guarded { child, pattern } => {
                json!({"anyOf": [self.render(child), {"type": "null"}], "pattern": pattern})
            }
            Node::Ref(child) => {
                let name = format!("Def{}x", self.definitions.len());
                self.definitions.insert(name.clone(), Value::Null);
                let rendered = self.render(child);
                self.definitions.insert(name.clone(), rendered);
                json!({"$ref": format!("#/components/schemas/{name}")})
            }
        }
    }
}

/// Drops the `null` members `json!` writes for absent options.
#[cfg(test)]
fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

/// An OpenRPC document whose single method takes `root` as its required param.
#[cfg(test)]
pub fn document(root: &Node) -> Value {
    let mut renderer = Renderer::default();
    let schema = renderer.render(root);
    strip_nulls(json!({
        "openrpc": "1.3.2",
        "methods": [{"name": "m", "params": [{"name": "p", "required": true, "schema": schema}], "result": {"name": "result", "schema": {"type": "object"}}}],
        "x-events": [],
        "x-notifications": [],
        "components": {"schemas": renderer.definitions},
    }))
}
