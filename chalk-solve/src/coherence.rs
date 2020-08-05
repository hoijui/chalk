use petgraph::prelude::*;

use crate::solve::Solver;
use crate::RustIrDatabase;
use chalk_ir::interner::Interner;
use chalk_ir::{self, ImplId, TraitId};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub mod orphan;
mod solve;

pub struct CoherenceSolver<'db, 'solver, I: Interner> {
    db: &'db dyn RustIrDatabase<I>,
    solver: &'solver mut dyn Solver<I>,
    trait_id: TraitId<I>,
}

#[derive(Debug)]
pub enum CoherenceError<I: Interner> {
    OverlappingImpls(TraitId<I>),
    FailedOrphanCheck(TraitId<I>),
}

impl<I: Interner> fmt::Display for CoherenceError<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoherenceError::OverlappingImpls(id) => {
                write!(f, "overlapping impls of trait `{:?}`", id)
            }
            CoherenceError::FailedOrphanCheck(id) => {
                write!(f, "impl for trait `{:?}` violates the orphan rules", id)
            }
        }
    }
}

impl<I: Interner> std::error::Error for CoherenceError<I> {}

/// Stores the specialization priorities for a set of impls.
/// This basically encodes which impls specialize one another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecializationPriorities<I: Interner> {
    map: BTreeMap<ImplId<I>, SpecializationPriority>,
}

impl<I: Interner> SpecializationPriorities<I> {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Lookup the priority of an impl in the set (panics if impl is not in set).
    pub fn priority(&self, impl_id: ImplId<I>) -> SpecializationPriority {
        self.map[&impl_id]
    }

    /// Store the priority of an impl (used during construction).
    /// Panics if we have already stored the priority for this impl.
    fn insert(&mut self, impl_id: ImplId<I>, p: SpecializationPriority) {
        let old_value = self.map.insert(impl_id, p);
        assert!(old_value.is_none());
    }
}

/// Impls with higher priority take precedence over impls with lower
/// priority (if both apply to the same types). Impls with equal
/// priority should never apply to the same set of input types.
#[derive(Copy, Clone, Default, PartialOrd, Ord, PartialEq, Eq, Debug)]
pub struct SpecializationPriority(usize);

impl<'db, 'solver, I> CoherenceSolver<'db, 'solver, I>
where
    I: Interner,
{
    /// Constructs a new `CoherenceSolver`.
    pub fn new(
        db: &'db dyn RustIrDatabase<I>,
        solver: &'solver mut dyn Solver<I>,
        trait_id: TraitId<I>,
    ) -> Self {
        Self {
            db,
            solver,
            trait_id,
        }
    }

    pub fn specialization_priorities(
        &mut self,
    ) -> Result<Arc<SpecializationPriorities<I>>, CoherenceError<I>> {
        let mut result = SpecializationPriorities::<I>::new();

        let forest = self.build_specialization_forest()?;

        // Visit every root in the forest & set specialization
        // priority for the tree that is the root of.
        for root_idx in forest.externals(Direction::Incoming) {
            self.set_priorities(root_idx, &forest, 0, &mut result);
        }

        Ok(Arc::new(result))
    }

    // Build the forest of specialization relationships.
    fn build_specialization_forest(&mut self) -> Result<Graph<ImplId<I>, ()>, CoherenceError<I>> {
        // The forest is returned as a graph but built as a GraphMap; this is
        // so that we never add multiple nodes with the same ItemId.
        let mut forest = DiGraphMap::new();

        // Find all specializations (implemented in coherence/solve)
        // Record them in the forest by adding an edge from the less special
        // to the more special.
        self.visit_specializations_of_trait(|less_special, more_special| {
            forest.add_edge(less_special, more_special, ());
        })?;

        Ok(forest.into_graph())
    }

    // Recursively set priorities for those node and all of its children.
    fn set_priorities(
        &self,
        idx: NodeIndex,
        forest: &Graph<ImplId<I>, ()>,
        p: usize,
        map: &mut SpecializationPriorities<I>,
    ) {
        // Get the impl datum recorded at this node and reset its priority
        {
            let impl_id = forest
                .node_weight(idx)
                .expect("index should be a valid index into graph");
            map.insert(*impl_id, SpecializationPriority(p));
        }

        // Visit all children of this node, setting their priority to this + 1
        for child_idx in forest.neighbors(idx) {
            self.set_priorities(child_idx, forest, p + 1, map);
        }
    }
}
