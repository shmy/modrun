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
}

#[derive(Debug)]
pub(crate) struct ScopeTree {
    nodes: Vec<ScopeNode>,
    /// Precomputed ancestor chains: `start`, parent, …, [`ScopeId::ROOT`].
    /// Extended incrementally in [`child`](Self::child).
    ancestors: Vec<Box<[ScopeId]>>,
}

impl ScopeTree {
    pub(crate) fn new() -> Self {
        Self {
            nodes: vec![ScopeNode { name: "<root>" }],
            ancestors: vec![Box::new([ScopeId::ROOT])],
        }
    }

    pub(crate) fn child(&mut self, parent: ScopeId, name: &'static str) -> ScopeId {
        let id = ScopeId(u32::try_from(self.nodes.len()).expect("scope id space exhausted"));
        self.nodes.push(ScopeNode { name });
        let mut chain = Vec::with_capacity(self.ancestors[parent.index()].len() + 1);
        chain.push(id);
        chain.extend_from_slice(&self.ancestors[parent.index()]);
        self.ancestors.push(chain.into_boxed_slice());
        id
    }

    pub(crate) fn name(&self, id: ScopeId) -> &'static str {
        self.nodes
            .get(id.index())
            .map(|n| n.name)
            .unwrap_or("<unknown>")
    }

    /// Walk from `start` up to root (inclusive). Uses a precomputed chain per scope.
    pub(crate) fn ancestors_from(&self, start: ScopeId) -> AncestorIter<'_> {
        AncestorIter::Cached {
            chain: &self.ancestors[start.index()],
            pos: 0,
        }
    }
}

/// Iterator over ancestor scopes from a starting scope up to root.
pub(crate) enum AncestorIter<'a> {
    Cached { chain: &'a [ScopeId], pos: usize },
}

impl Iterator for AncestorIter<'_> {
    type Item = ScopeId;

    fn next(&mut self) -> Option<Self::Item> {
        let Self::Cached { chain, pos } = self;
        if *pos >= chain.len() {
            return None;
        }
        let scope = chain[*pos];
        *pos += 1;
        Some(scope)
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
