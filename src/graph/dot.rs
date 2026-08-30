use std::any::TypeId;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::container::{Container, GroupElementKey, ProviderKey};
use crate::invoke::ScopedInvoker;
use crate::lifecycle::Lifecycle;
use crate::scope::ScopeId;
use crate::shutdown::Shutdowner;
use crate::trace::TARGET;

/// Render the dependency graph as a Graphviz DOT document.
pub(crate) fn render_dot(container: &Container, invokers: &[ScopedInvoker]) -> String {
    let renderer = DotRenderer::new(container, invokers);
    renderer.render()
}

struct DotRenderer<'a> {
    container: &'a Container,
    invokers: &'a [ScopedInvoker],
    nodes: Vec<NodeInfo>,
    keys: BTreeMap<String, usize>,
    edges: Vec<Edge>,
}

struct NodeInfo {
    scope: ScopeId,
    attrs: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeStyle {
    Dependency,
    GroupMember,
}

struct Edge {
    from: usize,
    to: usize,
    style: EdgeStyle,
}

impl<'a> DotRenderer<'a> {
    fn new(container: &'a Container, invokers: &'a [ScopedInvoker]) -> Self {
        Self {
            container,
            invokers,
            nodes: Vec::new(),
            keys: BTreeMap::new(),
            edges: Vec::new(),
        }
    }

    fn render(mut self) -> String {
        self.collect_nodes();
        self.collect_dependency_edges();
        self.collect_group_edges();
        self.format_dot()
    }

    fn collect_nodes(&mut self) {
        for &key in &self.container.provider_order {
            let Some(provider) = self.container.provider_at(key) else {
                continue;
            };
            let node_key = self.provider_node_key(key);
            let label = if key.is_group_member() {
                format!(
                    "{}\nctor={}\n(group member)",
                    label_type_name(provider.result_name()),
                    short_name(provider.constructor_name()),
                )
            } else if self.container.group_virtual_to_element.contains_key(&key) {
                format!(
                    "{}\nctor={}\n(group aggregate)",
                    display_type_name(provider.result_name()),
                    provider.constructor_name(),
                )
            } else {
                let mut label = format!(
                    "{}\nctor={}",
                    label_type_name(provider.result_name()),
                    short_name(provider.constructor_name()),
                );
                if key.private {
                    label.push_str("\n(private)");
                }
                label
            };
            self.ensure_node(&node_key, key.scope, &label);
        }

        for node in &self.container.value_nodes {
            if is_framework_dep(node.type_id) {
                continue;
            }
            let key = self.value_node_key(node.type_id, node.scope, node.private);
            let label = format!("{}\n(supplied)", label_type_name(node.type_name));
            self.ensure_node(&key, node.scope, &label);
        }

        for (index, scoped) in self.invokers.iter().enumerate() {
            let function = scoped.invoker.name();
            let key = self.invoker_node_key(index, function);
            let label = format!("{}\n(invoker)", short_name(function));
            self.ensure_node(&key, scoped.scope, &label);
        }
    }

    fn collect_dependency_edges(&mut self) {
        for &key in &self.container.provider_order {
            let Some(provider) = self.container.provider_at(key) else {
                continue;
            };
            let node = self.node_index(&self.provider_node_key(key));
            for &(dep_id, dep_name) in provider.dep_types() {
                if is_framework_dep(dep_id) {
                    continue;
                }
                if let Some(target) = self.resolve_dep(dep_id, dep_name, key.scope) {
                    self.edges.push(Edge {
                        from: target,
                        to: node,
                        style: EdgeStyle::Dependency,
                    });
                }
            }
        }

        for (index, scoped) in self.invokers.iter().enumerate() {
            let node = self.node_index(&self.invoker_node_key(index, scoped.invoker.name()));
            for &(dep_id, dep_name) in scoped.invoker.dep_types() {
                if is_framework_dep(dep_id) {
                    continue;
                }
                if let Some(target) = self.resolve_dep(dep_id, dep_name, scoped.scope) {
                    self.edges.push(Edge {
                        from: target,
                        to: node,
                        style: EdgeStyle::Dependency,
                    });
                }
            }
        }
    }

    fn collect_group_edges(&mut self) {
        for reg in self.container.group_registrations.values() {
            let group = self.node_index(&self.provider_node_key(reg.virtual_key));
            let members = self
                .container
                .group_members
                .get(&GroupElementKey {
                    element: reg.element,
                })
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for &member_key in members {
                let member = self.node_index(&self.provider_node_key(member_key));
                self.edges.push(Edge {
                    from: member,
                    to: group,
                    style: EdgeStyle::GroupMember,
                });
            }
        }
    }

    fn ensure_node(&mut self, key: &str, scope: ScopeId, label: &str) -> usize {
        if let Some(&existing) = self.keys.get(key) {
            if self.nodes[existing].scope != scope {
                tracing::warn!(
                    target: TARGET,
                    key,
                    "modrun graph node key reused across scopes: {key}"
                );
            }
            return existing;
        }

        let cluster = self.container.scopes().name(scope);
        let attrs = format!(
            "label=\"{}\" tooltip=\"{}\"",
            dot_escape(label),
            dot_escape(&format!("{cluster}::{key}"))
        );
        let id = self.nodes.len();
        self.nodes.push(NodeInfo { scope, attrs });
        self.keys.insert(key.to_owned(), id);
        id
    }

    fn node_index(&self, key: &str) -> usize {
        *self
            .keys
            .get(key)
            .unwrap_or_else(|| panic!("graph node missing for key: {key}"))
    }

    fn resolve_dep(&self, dep_id: TypeId, dep_name: &'static str, from: ScopeId) -> Option<usize> {
        let key = if let Some((scope, private)) = self.container.resolve_value_binding(dep_id, from)
        {
            self.value_node_key(dep_id, scope, private)
        } else if let Some((dep_key, _)) = self.container.resolve_provider(dep_id, from) {
            self.provider_node_key(dep_key)
        } else {
            tracing::warn!(
                target: TARGET,
                dependency = dep_name,
                module = self.container.scopes().name(from),
                "modrun graph could not resolve dependency edge for {dep_name}"
            );
            return None;
        };

        if self.keys.contains_key(&key) {
            return Some(self.node_index(&key));
        }

        tracing::warn!(
            target: TARGET,
            dependency = dep_name,
            module = self.container.scopes().name(from),
            "modrun graph dependency resolved but node is missing: {dep_name}"
        );
        None
    }

    fn provider_node_key(&self, key: ProviderKey) -> String {
        let provider = self
            .container
            .provider_at(key)
            .expect("provider node missing provider");
        let module = self.container.scopes().name(key.scope);
        if key.is_group_member() {
            return format!(
                "provider:{module}:{}#{}",
                short_name(provider.constructor_name()),
                key.ordinal
            );
        }
        let type_name = if self.container.group_virtual_to_element.contains_key(&key) {
            display_type_name(provider.result_name())
        } else {
            label_type_name(provider.result_name())
        };
        if key.private {
            format!("provider:{module}:{type_name}:private")
        } else {
            format!("provider:{module}:{type_name}")
        }
    }

    fn value_node_key(&self, type_id: TypeId, scope: ScopeId, private: bool) -> String {
        let module = self.container.scopes().name(scope);
        let privacy = if private { "private" } else { "public" };
        format!("value:{module}:{type_id:?}:{privacy}")
    }

    fn invoker_node_key(&self, index: usize, function: &'static str) -> String {
        format!("invoker:{index}:{}", short_name(function))
    }

    fn format_dot(&self) -> String {
        let mut out = String::from("digraph modrun {\n  rankdir=LR;\n  node [shape=box];\n");
        let mut clusters: BTreeMap<ScopeId, Vec<usize>> = BTreeMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            clusters.entry(node.scope).or_default().push(index);
        }

        for (scope_id, node_indices) in clusters {
            let scope_name = self.container.scopes().name(scope_id);
            if scope_id == ScopeId::ROOT {
                for index in node_indices {
                    writeln!(out, "  n{index} [{}];", self.nodes[index].attrs).unwrap();
                }
                continue;
            }
            writeln!(out, "  subgraph cluster_{} {{", scope_id.discriminant()).unwrap();
            writeln!(out, "    label=\"{}\";", dot_escape(scope_name)).unwrap();
            for index in node_indices {
                writeln!(out, "    n{index} [{}];", self.nodes[index].attrs).unwrap();
            }
            writeln!(out, "  }}").unwrap();
        }

        for edge in &self.edges {
            let style = match edge.style {
                EdgeStyle::Dependency => "",
                EdgeStyle::GroupMember => " [style=dotted, arrowhead=none]",
            };
            writeln!(out, "  n{} -> n{}{style};", edge.from, edge.to).unwrap();
        }
        out.push_str("}\n");
        out
    }
}

fn display_type_name(full: &str) -> String {
    full.strip_prefix("modrun::").unwrap_or(full).to_owned()
}

fn label_type_name(full: &str) -> String {
    let trimmed = full
        .strip_prefix("alloc::sync::")
        .or_else(|| full.strip_prefix("std::sync::"))
        .unwrap_or(full);
    let trimmed = trimmed.strip_prefix("modrun::").unwrap_or(trimmed);
    if trimmed.contains('<') {
        trimmed.to_owned()
    } else {
        trimmed.rsplit("::").next().unwrap_or(trimmed).to_owned()
    }
}

fn short_name(full: &str) -> &str {
    full.rsplit("::").next().unwrap_or(full)
}

fn dot_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn is_framework_dep(id: TypeId) -> bool {
    id == TypeId::of::<Lifecycle>()
        || id == TypeId::of::<Shutdowner>()
        || id == TypeId::of::<Arc<Lifecycle>>()
        || id == TypeId::of::<Arc<Shutdowner>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_escape_handles_special_characters() {
        let escaped = dot_escape("a\"b\\c\nd");
        assert_eq!(escaped, "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn label_type_name_handles_arc_generics() {
        assert_eq!(
            label_type_name("alloc::sync::Arc<graph::Config>"),
            "Arc<graph::Config>"
        );
        assert_eq!(label_type_name("graph::Config"), "Config");
        assert_eq!(
            label_type_name("my::Wrapper<my::Config>"),
            "my::Wrapper<my::Config>"
        );
    }

    #[test]
    fn is_framework_dep_matches_lifecycle_and_shutdowner() {
        assert!(is_framework_dep(TypeId::of::<Lifecycle>()));
        assert!(is_framework_dep(TypeId::of::<Shutdowner>()));
        assert!(is_framework_dep(TypeId::of::<Arc<Lifecycle>>()));
        assert!(is_framework_dep(TypeId::of::<Arc<Shutdowner>>()));
        assert!(!is_framework_dep(TypeId::of::<u32>()));
    }
}
