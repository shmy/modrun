/// Identifies a module scope in the application tree. Root is [`ScopeId::ROOT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ScopeId(u32);

impl ScopeId {
    pub(crate) const ROOT: ScopeId = ScopeId(0);

    pub(crate) fn discriminant(self) -> u32 {
        self.0
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug)]
struct ScopeNode {
    name: &'static str,
    parent: Option<ScopeId>,
}

#[derive(Debug)]
pub(crate) struct ScopeTree {
    nodes: Vec<ScopeNode>,
}

impl ScopeTree {
    pub(crate) fn new() -> Self {
        Self {
            nodes: vec![ScopeNode {
                name: "<root>",
                parent: None,
            }],
        }
    }

    pub(crate) fn child(&mut self, parent: ScopeId, name: &'static str) -> ScopeId {
        let id = ScopeId(u32::try_from(self.nodes.len()).expect("scope id space exhausted"));
        self.nodes.push(ScopeNode {
            name,
            parent: Some(parent),
        });
        id
    }

    pub(crate) fn parent(&self, id: ScopeId) -> Option<ScopeId> {
        self.nodes.get(id.index()).and_then(|n| n.parent)
    }

    pub(crate) fn name(&self, id: ScopeId) -> &'static str {
        self.nodes
            .get(id.index())
            .map(|n| n.name)
            .unwrap_or("<unknown>")
    }

    /// Walk from `start` up to root (inclusive), without allocating.
    pub(crate) fn ancestors_from(&self, start: ScopeId) -> Ancestors<'_> {
        Ancestors {
            tree: self,
            current: Some(start),
        }
    }
}

/// Iterator over ancestor scopes from a starting scope up to root.
pub(crate) struct Ancestors<'a> {
    tree: &'a ScopeTree,
    current: Option<ScopeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = ScopeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.current?;
        self.current = self.tree.parent(id);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestors_walk_to_root() {
        let mut tree = ScopeTree::new();
        let a = tree.child(ScopeId::ROOT, "a");
        let b = tree.child(a, "b");
        let got: Vec<_> = tree.ancestors_from(b).collect();
        assert_eq!(got, vec![b, a, ScopeId::ROOT]);
        assert_eq!(tree.name(b), "b");
    }
}
